import Foundation

/// A push notification category, mirroring the gateway's `push_policy.rs`
/// category families. Preview controls are set per category so the operator can,
/// say, always show job-progress previews but keep approval detail off the lock
/// screen (docs/MOBILE_SECURITY.md **D-N3**).
public enum NotificationCategory: String, CaseIterable, Hashable, Sendable {
    case message
    case approval
    case job
}

/// How much of a push's real content the Notification Service Extension may
/// reveal for a category (docs/MOBILE_SECURITY.md **D-N3**: previews
/// always / when-unlocked / never). This gates the *local* content rewrite the
/// NSE performs — the APNs payload itself is always content-free (D-N1), so
/// "never" simply leaves the generic placeholder text in place.
public enum PreviewMode: String, CaseIterable, Hashable, Sendable {
    /// Rewrite with real content whenever it can be fetched, locked or not.
    case always
    /// Rewrite only while the device is unlocked; keep the generic text on the
    /// lock screen. The default — matches iOS's own "when unlocked" preview
    /// posture.
    case whenUnlocked
    /// Never rewrite: the generic placeholder always stands.
    case never
    /// Approvals-only refinement: deliver a content-free alert that surfaces
    /// only inside the app (no lock-screen/banner detail). Modeled as a distinct
    /// mode so the NSE treats it like `never` for the rewrite decision while the
    /// UI can still label it "App only".
    case appOnly

    /// Whether the NSE may perform its local content rewrite for this mode given
    /// the current lock state. `appOnly`/`never` never rewrite; `whenUnlocked`
    /// rewrites only when unlocked; `always` always rewrites.
    ///
    /// The NSE cannot itself always tell whether the device is unlocked (it runs
    /// in a background extension), so `deviceUnlocked` is supplied by the caller;
    /// when it is `nil` (unknown) `whenUnlocked` fails closed to *no* rewrite.
    public func allowsRewrite(deviceUnlocked: Bool?) -> Bool {
        switch self {
        case .always: return true
        case .never, .appOnly: return false
        case .whenUnlocked: return deviceUnlocked == true
        }
    }
}

/// The operator's per-category notification preview preferences (D-N3),
/// persisted in the App Group so the Notification Service Extension reads the
/// same values the settings UI writes.
///
/// Only the `approval` category may be set to ``PreviewMode/appOnly``; the model
/// enforces that so the UI and the NSE agree on the valid set without a separate
/// validator.
public struct NotificationPreferences: Hashable, Sendable {
    /// Preview mode for messages. `appOnly` is coerced to `never` (not a valid
    /// message option).
    public var message: PreviewMode
    /// Preview mode for approvals. The only category that supports `appOnly`.
    public var approval: PreviewMode
    /// Preview mode for jobs. `appOnly` is coerced to `never`.
    public var job: PreviewMode

    public init(
        message: PreviewMode = .whenUnlocked,
        approval: PreviewMode = .whenUnlocked,
        job: PreviewMode = .whenUnlocked
    ) {
        self.message = Self.sanitize(message, for: .message)
        self.approval = Self.sanitize(approval, for: .approval)
        self.job = Self.sanitize(job, for: .job)
    }

    /// The default posture: previews only when unlocked, on every category.
    public static let `default` = NotificationPreferences()

    /// The valid preview modes for a category. Only approvals offer `appOnly`.
    public static func allowedModes(for category: NotificationCategory) -> [PreviewMode] {
        switch category {
        case .approval:
            return [.always, .whenUnlocked, .appOnly, .never]
        case .message, .job:
            return [.always, .whenUnlocked, .never]
        }
    }

    /// The preview mode currently set for `category`.
    public func mode(for category: NotificationCategory) -> PreviewMode {
        switch category {
        case .message: return message
        case .approval: return approval
        case .job: return job
        }
    }

    /// Return a copy with `category` set to `mode` (coercing an invalid mode to
    /// the nearest valid one for that category).
    public func setting(_ mode: PreviewMode, for category: NotificationCategory) -> Self {
        var copy = self
        let clean = Self.sanitize(mode, for: category)
        switch category {
        case .message: copy.message = clean
        case .approval: copy.approval = clean
        case .job: copy.job = clean
        }
        return copy
    }

    /// Coerce a mode to one valid for the category: `appOnly` on a non-approval
    /// category collapses to `never` (the most private valid option).
    private static func sanitize(_ mode: PreviewMode, for category: NotificationCategory) -> PreviewMode {
        if mode == .appOnly, category != .approval { return .never }
        return mode
    }
}
