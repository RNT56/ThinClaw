import Testing
import UserNotifications

@Suite("Notification service completion")
struct NotificationCompletionTests {
    @Test("finish consumes the content handler exactly once")
    func completionOnce() {
        let gate = NotificationCompletionGate()
        let content = UNMutableNotificationContent()
        content.title = "Redacted"
        var calls = 0
        gate.install(handler: { _ in calls += 1 }, bestAttempt: content)

        gate.finish()
        gate.finish()
        gate.cancelAndFinish()

        #expect(calls == 1)
    }

    @Test("expiry preserves the best available rewritten content")
    func expiryPreservesBestAttempt() {
        let gate = NotificationCompletionGate()
        let content = UNMutableNotificationContent()
        content.title = "Redacted"
        var deliveredTitle: String?
        gate.install(
            handler: { deliveredTitle = $0.title },
            bestAttempt: content)
        gate.apply(title: "Approval", body: "Review in ThinClaw")

        gate.cancelAndFinish()

        #expect(deliveredTitle == "Approval")
    }
}

@Suite("Notification preview policy")
struct NotificationPreviewPolicyTests {
    @Test("locked when-unlocked policy fails closed")
    func lockedFailsClosed() {
        #expect(!NotificationPreviewPreference.Mode.whenUnlocked.allowsRewrite { false })
        #expect(NotificationPreviewPreference.Mode.whenUnlocked.allowsRewrite { true })
    }

    @Test("always and redacted modes ignore lock state")
    func fixedPolicies() {
        #expect(NotificationPreviewPreference.Mode.always.allowsRewrite { false })
        #expect(!NotificationPreviewPreference.Mode.never.allowsRewrite { true })
        #expect(!NotificationPreviewPreference.Mode.appOnly.allowsRewrite { true })
    }
}

@Suite("Notification payload parsing")
struct NotificationPayloadTests {
    @Test("malformed payload safely produces no identifiers")
    func malformedPayload() {
        let ids = PushIDs(userInfo: ["tc": "not-a-dictionary"])
        #expect(ids.requestID == nil)
        #expect(ids.threadID == nil)
        #expect(ids.jobID == nil)
    }
}
