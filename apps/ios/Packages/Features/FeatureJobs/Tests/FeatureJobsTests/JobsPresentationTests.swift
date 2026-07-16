import Testing
import ThinClawCore

@testable import FeatureJobs

@Suite("Jobs presentation")
struct JobsPresentationTests {
    @Test(
        "every server phase has a stable native symbol",
        arguments: [
            (JobPhase.pending, "clock"),
            (.running, "arrow.triangle.2.circlepath"),
            (.succeeded, "checkmark.circle.fill"),
            (.failed, "xmark.octagon.fill"),
            (.cancelled, "slash.circle"),
            (.stuck, "exclamationmark.triangle.fill"),
            (.unknown, "questionmark.circle"),
        ])
    func phaseSymbol(_ phase: JobPhase, _ expected: String) {
        #expect(phase.symbolName == expected)
    }
}
