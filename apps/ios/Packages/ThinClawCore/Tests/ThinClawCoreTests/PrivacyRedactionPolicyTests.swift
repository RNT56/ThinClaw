import Testing

@testable import ThinClawCore

@Suite("PrivacyRedactionPolicy")
struct PrivacyRedactionPolicyTests {
    typealias Phase = PrivacyRedactionPolicy.Phase

    @Test("every non-active phase is redacted (always-on switcher overlay)")
    func redactsNonActive() {
        #expect(!PrivacyRedactionPolicy.shouldRedact(for: .active))
        #expect(PrivacyRedactionPolicy.shouldRedact(for: .inactive))
        #expect(PrivacyRedactionPolicy.shouldRedact(for: .background))
    }

    @Test("the app-switcher snapshot phase (.inactive) is covered")
    func coversSwitcherSnapshot() {
        // iOS composites the multitasking snapshot while the scene is
        // .inactive; that is the phase that must never show the transcript.
        #expect(PrivacyRedactionPolicy.shouldRedact(for: .inactive))
    }

    @Test("active is never redacted")
    func activeNeverRedacts() {
        #expect(!PrivacyRedactionPolicy.shouldRedact(for: .active))
    }
}
