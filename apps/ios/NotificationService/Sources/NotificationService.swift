import Foundation
import Security
import ThinClawAPI
import ThinClawAuth
import UserNotifications

/// Notification Service Extension (docs/MOBILE_SECURITY.md **D-N1**).
///
/// Every ThinClaw push arrives content-free: a generic `aps.alert` with
/// `mutable-content: 1` and an id-only `tc` dict (`request_id` / `thread_id` /
/// `job_id`). This extension gets a short window to *rewrite* the visible
/// title/body by fetching the real content from the gateway over the **same
/// pinned connection** the app uses, reading the device token from the shared
/// Keychain access group. If the gateway is unreachable (locked-out network,
/// gateway down) the generic text stands — content is never shipped through
/// APNs.
///
/// The extension links only what it needs: ``ThinClawAuth`` (shared Keychain +
/// TLS pinning) and ``ThinClawAPI`` (generated REST client). It intentionally
/// does not touch the app's feature graph.
final class NotificationService: UNNotificationServiceExtension {
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        let mutable = request.content.mutableCopy() as? UNMutableNotificationContent
        bestAttempt = mutable

        guard let mutable else {
            contentHandler(request.content)
            return
        }

        let ids = PushIDs(userInfo: request.content.userInfo)

        Task {
            if let rewrite = await Self.fetchRewrite(
                category: request.content.categoryIdentifier, ids: ids)
            {
                mutable.title = rewrite.title
                mutable.body = rewrite.body
            }
            // Either way, deliver: on success with local content, otherwise with
            // the generic text APNs carried.
            contentHandler(mutable)
        }
    }

    /// The system is about to kill the extension (time budget spent): hand back
    /// the best content we have, which is at worst the generic APNs text.
    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let bestAttempt {
            contentHandler(bestAttempt)
        }
    }

    /// Fetch the real content for this push from the gateway and return the
    /// rewritten title/body, or `nil` to leave the generic text (unpaired,
    /// unreachable, or nothing to show for this category).
    private static func fetchRewrite(
        category: String, ids: PushIDs
    ) async -> (title: String, body: String)? {
        guard let credential = SharedGatewayConnection.loadCredential(),
            let baseURL = credential.preferredBaseURL
        else { return nil }

        let session = SharedGatewayConnection.pinnedSession(for: credential)
        let token = credential.deviceToken
        let client = GatewayClient.make(baseURL: baseURL, token: { token }, session: session)

        switch category {
        case categoryApprovalLow, categoryApprovalHigh, categoryApproval:
            // D-N3: honor the operator's per-category preview preference. When the
            // approval preview is `never`/`app_only`, or `when_unlocked` while the
            // device is likely locked, leave the generic content-free text.
            guard NotificationPreviewPreference.load(for: .approval).allowsRewrite() else {
                return nil
            }
            return await approvalRewrite(client: client, requestID: ids.requestID)
        default:
            // Messages/jobs have no dedicated content-fetch endpoint in the v1
            // contract yet, so leave the generic text rather than pulling whole
            // thread history into the extension (a heavier fetch than the small
            // NSE budget wants). Approvals are the high-value rewrite.
            return nil
        }
    }

    /// Look up the pending approval by `request_id` and render a tool-named
    /// title + description. Uses the best-effort `GET /api/chat/approvals` pull
    /// endpoint (the same one the in-app approvals surface falls back to).
    private static func approvalRewrite(
        client: Client, requestID: String?
    ) async -> (title: String, body: String)? {
        guard let requestID else { return nil }
        guard
            let response = try? await client.chatApprovalsHandler(),
            case let .ok(ok) = response,
            case let .json(payload) = ok.body,
            let entry = payload.approvals.first(where: { $0.requestId == requestID })
        else { return nil }

        let title = "Approve \(entry.toolName)?"
        let body = entry.description.isEmpty ? "Tap to review this request." : entry.description
        return (title, body)
    }
}

/// The id-only payload the gateway ships under the `tc` dict (D-N1); mirrors the
/// app-side reader. Kept local so the extension links no app code.
private struct PushIDs {
    let requestID: String?
    let threadID: String?
    let jobID: String?

    init(userInfo: [AnyHashable: Any]) {
        let tc = userInfo["tc"] as? [String: Any]
        requestID = tc?["request_id"] as? String
        threadID = tc?["thread_id"] as? String
        jobID = tc?["job_id"] as? String
    }
}

// The approval APNs category identifiers, mirrored from `push_policy.rs` so the
// extension links no app target. Only approvals get a local content rewrite
// (D-N1); message/job pushes fall through to the generic text, so their
// categories are not needed here.
private let categoryApproval = "THINCLAW_APPROVAL"
private let categoryApprovalLow = "THINCLAW_APPROVAL_LOW"
private let categoryApprovalHigh = "THINCLAW_APPROVAL_HIGH"

/// The operator's per-category preview preference (docs/MOBILE_SECURITY.md
/// **D-N3**), read straight from the shared App Group defaults so the extension
/// links no app code. The keys + raw values are the stable cross-process
/// contract written by `ThinClawCore.NotificationPreferencesStore`; kept local
/// here to keep the NSE's dependency surface minimal.
private enum NotificationPreviewPreference {
    /// Push categories the operator can gate. Only `approval` is consulted in the
    /// NSE today (the only category with a content rewrite), but the enum mirrors
    /// the full model so the key contract stays honest.
    enum Category: String {
        case message
        case approval
        case job
    }

    /// Preview modes, mirroring `ThinClawCore.PreviewMode` raw values exactly.
    enum Mode: String {
        case always
        case whenUnlocked
        case never
        case appOnly
    }

    /// The App Group suite the app + NSE share (D-K2).
    private static let appGroupID = "group.com.thinclaw.shared"

    /// Load the persisted mode for `category`, defaulting to `whenUnlocked` when
    /// unset/unrecognized — the same default the app-side store applies.
    static func load(for category: Category) -> Mode {
        guard
            let defaults = UserDefaults(suiteName: appGroupID),
            let raw = defaults.string(forKey: "notif.preview.\(category.rawValue)"),
            let mode = Mode(rawValue: raw)
        else {
            return .whenUnlocked
        }
        return mode
    }
}

extension NotificationPreviewPreference.Mode {
    /// Whether the NSE may perform its local content rewrite under this mode.
    /// `never`/`appOnly` never rewrite; `whenUnlocked` rewrites only while the
    /// device is unlocked (probed via ``deviceIsUnlocked``, failing closed when
    /// the probe is inconclusive); `always` always rewrites.
    func allowsRewrite() -> Bool {
        switch self {
        case .always:
            return true
        case .never, .appOnly:
            return false
        case .whenUnlocked:
            return deviceIsUnlocked()
        }
    }
}

/// Best-effort device-unlocked probe for the NSE: attempt to read a Keychain
/// item classed `WhenUnlockedThisDeviceOnly`. A successful read means the device
/// is unlocked; `errSecInteractionNotAllowed` means locked. Any other error (or
/// a missing probe item — never provisioned) is treated as **locked** so the
/// `whenUnlocked` preview fails closed to the generic content-free text.
///
/// The probe item is optional infrastructure: when absent the NSE simply keeps
/// previews off for `whenUnlocked`, which is the private-by-default outcome.
private func deviceIsUnlocked() -> Bool {
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "com.thinclaw.lockprobe",
        kSecReturnData as String: false,
        kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    let status = SecItemCopyMatching(query as CFDictionary, nil)
    switch status {
    case errSecSuccess:
        return true
    default:
        // errSecInteractionNotAllowed (locked), errSecItemNotFound (no probe),
        // or anything else → treat as locked / fail closed.
        return false
    }
}
