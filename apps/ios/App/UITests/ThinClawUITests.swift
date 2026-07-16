import XCTest

final class ThinClawUITests: XCTestCase {
    @MainActor
    func testUnpairedOnboardingIsAccessible() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-unpaired", "--uitesting-light"]
        app.launch()

        let title = app.staticTexts["onboarding.title"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["onboarding.manual"].isHittable)
        try app.performAccessibilityAudit(
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
        try app.performAccessibilityAudit(for: [.contrast, .textClipped])

        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "onboarding-dark-accessibility-xxxl"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
