import Testing
import ThinClawCore
import ThinClawDesign

@testable import FeatureApprovals

@Suite("Approvals presentation")
struct ApprovalsPresentationTests {
    @Test("domain risk tiers preserve their authorization severity")
    func riskTierMapping() {
        #expect(RiskTier.low.designTier == ApprovalCard.RiskTier.low)
        #expect(RiskTier.high.designTier == ApprovalCard.RiskTier.high)
    }
}
