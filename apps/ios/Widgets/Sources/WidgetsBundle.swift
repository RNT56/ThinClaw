import SwiftUI
import WidgetKit

@main
struct ThinClawWidgetsBundle: WidgetBundle {
    var body: some Widget {
        AgentStatusWidget()
        PendingApprovalsWidget()
        QuickAskWidget()
        AgentRunLiveActivity()
    }
}
