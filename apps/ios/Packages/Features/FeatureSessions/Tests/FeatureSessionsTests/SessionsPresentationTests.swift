import Testing
import ThinClawCore

@testable import FeatureSessions

@Suite("Sessions presentation")
struct SessionsPresentationTests {
    @Test("routed thread identifiers remain deterministic and untruncated")
    func accessibilityIdentifier() {
        let threadID = ThreadID("gateway/thread with spaces")
        #expect(
            SessionsPresentation.accessibilityIdentifier(for: threadID)
                == "session.gateway/thread with spaces")
    }
}
