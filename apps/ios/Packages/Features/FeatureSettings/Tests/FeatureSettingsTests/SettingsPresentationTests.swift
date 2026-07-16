import Testing
import ThinClawCore

@testable import FeatureSettings

@Suite("Settings presentation")
struct SettingsPresentationTests {
    @Test(
        "notification category labels are complete",
        arguments: [
            (NotificationCategory.message, "Messages"),
            (.approval, "Approvals"),
            (.job, "Jobs"),
        ])
    func categoryLabels(_ category: NotificationCategory, _ expected: String) {
        #expect(SettingsScreen.categoryTitle(category) == expected)
    }

    @Test(
        "preview policy labels describe each privacy mode",
        arguments: [
            (PreviewMode.always, "Always"),
            (.whenUnlocked, "When unlocked"),
            (.never, "Never"),
            (.appOnly, "App only"),
        ])
    func previewLabels(_ mode: PreviewMode, _ expected: String) {
        #expect(SettingsScreen.previewModeTitle(mode) == expected)
    }
}
