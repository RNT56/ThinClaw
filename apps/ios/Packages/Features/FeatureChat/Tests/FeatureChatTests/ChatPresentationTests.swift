import Testing
import ThinClawCore
import ThinClawDesign

@testable import FeatureChat

@Suite("Chat presentation")
struct ChatPresentationTests {
    @Test(
        "transport states map to honest connection pills",
        arguments: [
            (ConnectionState.idle, StatusPill.Status.offline),
            (.connecting, .connecting),
            (.connected, .connected),
            (.reconnecting(attempt: 2), .connecting),
            (.failed(message: "offline"), .offline),
        ])
    @MainActor
    func connectionStateMapping(_ state: ConnectionState, _ expected: StatusPill.Status) {
        #expect(ChatStore.pillStatus(for: state) == expected)
    }
}
