import XCTest

final class ThinClawUITests: XCTestCase {
    private static let accessibilityAuditErrorDomain =
        "com.apple.xcode.xctest.accessibilityAudit"
    private static let accessibilityAuditTimeoutCode = -56

    @MainActor
    func testUnpairedOnboardingIsAccessible() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-unpaired", "--uitesting-light"]
        app.launch()

        let title = app.staticTexts["onboarding.title"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["onboarding.manual"].isHittable)
        try performAccessibilityAuditWithTimeoutRetry(
            on: app,
            for: [.contrast, .elementDetection, .hitRegion, .sufficientElementDescription])

        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = "onboarding-unpaired"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    func testOnboardingAtAccessibilityTextSizeInDarkMode() throws {
        let app = XCUIApplication()
        app.launchArguments = [
            "--uitesting-unpaired",
            "--uitesting-dark-accessibility",
        ]
        app.launch()

        XCTAssertTrue(app.staticTexts["onboarding.title"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["onboarding.manual"].isHittable)
        try performAccessibilityAuditWithTimeoutRetry(
            on: app,
            for: [.contrast, .textClipped])

        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "onboarding-dark-accessibility-xxxl"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    private func performAccessibilityAuditWithTimeoutRetry(
        on app: XCUIApplication,
        for auditTypes: XCUIAccessibilityAuditType
    ) throws {
        do {
            try app.performAccessibilityAudit(for: auditTypes)
        } catch {
            let auditError = error as NSError
            guard
                auditError.domain == Self.accessibilityAuditErrorDomain,
                auditError.code == Self.accessibilityAuditTimeoutCode
            else {
                throw error
            }

            // Xcode's audit service can time out independently of the app under test.
            // Retry that infrastructure failure once; real findings still fail immediately.
            app.activate()
            Thread.sleep(forTimeInterval: 1)
            try app.performAccessibilityAudit(for: auditTypes)
        }
    }
}
