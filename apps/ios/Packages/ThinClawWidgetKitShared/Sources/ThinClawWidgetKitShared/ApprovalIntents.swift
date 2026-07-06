import AppIntents
import Foundation
import ThinClawSnapshotKit

/// Approve a pending low-risk tool request directly from the widget process.
///
/// High-risk approvals are never offered here — the widget deep-links into
/// the app for biometric confirmation (docs/MOBILE_SECURITY.md, D-K3).
/// The POST is idempotent by `request_id` server-side, so retry after an
/// unreachable-gateway failure is safe.
public struct ApproveToolIntent: AppIntent {
    public static let title: LocalizedStringResource = "Approve tool run"
    public static let description = IntentDescription(
        "Approves a pending low-risk ThinClaw tool request.")

    @Parameter(title: "Request ID")
    public var requestID: String

    @Parameter(title: "Thread ID")
    public var threadID: String?

    /// The gateway-computed risk tier the widget stamped onto the button.
    /// Defaults to `"high"` so a decoding gap fails closed (D-K3): an inline
    /// approve for anything not explicitly `"low"` is refused here even if the
    /// widget somehow rendered the button.
    @Parameter(title: "Risk")
    public var risk: String

    public init() {}

    public init(requestID: String, threadID: String?, risk: String = "high") {
        self.requestID = requestID
        self.threadID = threadID
        self.risk = risk
    }

    public func perform() async throws -> some IntentResult {
        // D-K3 defense-in-depth: the lock screen must NEVER approve a
        // high-risk tool. The widget already omits the approve button for
        // high-risk rows, but we re-check the tier the button carried and
        // refuse anything that is not explicitly low-risk. High-risk approval
        // requires the in-app biometric gate.
        guard risk == "low" else {
            throw ApprovalGateError.highRiskApprovalRefused
        }
        #if canImport(Security) && canImport(CryptoKit)
            try await WidgetGatewayCall.submitApproval(
                requestID: requestID, threadID: threadID, action: "approve")
            WidgetReload.approvals()
        #endif
        return .result()
    }
}

/// Reasons an interactive approval intent refuses to act.
public enum ApprovalGateError: Error, CustomLocalizedStringResourceConvertible {
    /// A high-risk (or unknown-risk) entry cannot be approved off-device;
    /// open the app for the biometric gate (docs/MOBILE_SECURITY.md D-K3).
    case highRiskApprovalRefused

    public var localizedStringResource: LocalizedStringResource {
        switch self {
        case .highRiskApprovalRefused:
            "Open ThinClaw to approve this high-risk action."
        }
    }
}

/// Deny a pending tool request from the widget process.
public struct DenyToolIntent: AppIntent {
    public static let title: LocalizedStringResource = "Deny tool run"
    public static let description = IntentDescription(
        "Denies a pending ThinClaw tool request.")

    @Parameter(title: "Request ID")
    public var requestID: String

    @Parameter(title: "Thread ID")
    public var threadID: String?

    public init() {}

    public init(requestID: String, threadID: String?) {
        self.requestID = requestID
        self.threadID = threadID
    }

    public func perform() async throws -> some IntentResult {
        // Deny is never risk-gated (D-K3): declining an action is always safe
        // from any surface.
        #if canImport(Security) && canImport(CryptoKit)
            try await WidgetGatewayCall.submitApproval(
                requestID: requestID, threadID: threadID, action: "deny")
            WidgetReload.approvals()
        #endif
        return .result()
    }
}

/// Fire a prompt at the agent without opening the app; the answer arrives as
/// a push notification. Also exposed as an App Shortcut ("Ask ThinClaw…").
public struct QuickAskIntent: AppIntent {
    public static let title: LocalizedStringResource = "Ask ThinClaw"
    public static let description = IntentDescription(
        "Sends a prompt to your ThinClaw agent; the reply arrives as a notification.")

    @Parameter(title: "Prompt")
    public var prompt: String

    public init() {}

    public init(prompt: String) {
        self.prompt = prompt
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        // POST /api/chat/send over the pinned session; the completion push is
        // delivered by the gateway when this device holds no live stream. A
        // QuickAskReceipt is written so the widget can render "sent ✓".
        #if canImport(Security) && canImport(CryptoKit)
            let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else {
                return .result(dialog: "Nothing to send.")
            }
            do {
                try await WidgetGatewayCall.sendPrompt(trimmed, threadID: nil)
                writeReceipt(text: trimmed, state: .sent)
                return .result(dialog: "Sent to ThinClaw.")
            } catch {
                // Queue locally so the app can retry on next foreground, and
                // tell the user honestly that it did not go through yet.
                writeReceipt(text: trimmed, state: .failed)
                return .result(dialog: "Couldn't reach ThinClaw — saved to send later.")
            }
        #else
            return .result(dialog: "Sent to ThinClaw.")
        #endif
    }

    private func writeReceipt(text: String, state: QuickAskReceipt.DeliveryState) {
        guard let store = WidgetSnapshotAccess.store() else { return }
        let receipt = QuickAskReceipt(
            generatedAt: .now, text: text, threadID: nil, deliveryState: state)
        try? store.save(receipt)
        #if canImport(WidgetKit)
            WidgetReload.quickAsk()
        #endif
    }
}

/// Registers the Siri / Shortcuts phrases.
public struct ThinClawShortcuts: AppShortcutsProvider {
    public static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: QuickAskIntent(),
            phrases: ["Ask \(.applicationName)"],
            shortTitle: "Ask ThinClaw",
            systemImageName: "bubble.left.and.text.bubble.right"
        )
    }
}
