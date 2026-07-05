import AppIntents
import Foundation

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

    public init() {}

    public init(requestID: String, threadID: String?) {
        self.requestID = requestID
        self.threadID = threadID
    }

    public func perform() async throws -> some IntentResult {
        // M3: POST /api/chat/approval {request_id, action: "approve"} via the
        // shared-Keychain device token; rewrite approvals snapshot; reload
        // widget timelines. Scaffold returns without side effects.
        .result()
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
        // M3: POST /api/chat/approval {request_id, action: "deny"}.
        .result()
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
        // M3: POST /api/chat/send; completion push delivered by the gateway
        // when this device holds no live stream.
        .result(dialog: "Sent to ThinClaw.")
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
