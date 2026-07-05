import FeatureApprovals
import FeatureChat
import FeatureJobs
import FeatureOnboarding
import FeatureSessions
import FeatureSettings
import SwiftUI
import ThinClawDesign

@main
struct ThinClawApp: App {
    @State private var dependencies = AppDependencies()
    @State private var router = AppRouter()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(dependencies)
                .environment(router)
                .onOpenURL { url in
                    router.handle(deepLink: url)
                }
        }
    }
}

/// Tab shell (Chat / Sessions / Jobs / Settings). Approvals surface as a
/// badge + sheet from anywhere, not a tab. Onboarding replaces the shell
/// until a device credential exists.
struct RootView: View {
    @Environment(AppDependencies.self) private var dependencies
    @Environment(AppRouter.self) private var router

    var body: some View {
        @Bindable var router = router
        if dependencies.isPaired {
            TabView(selection: $router.selectedTab) {
                Tab("Chat", systemImage: "bubble.left.and.text.bubble.right", value: AppTab.chat) {
                    NavigationStack(path: $router.chatPath) {
                        ChatScreen()
                    }
                }
                Tab("Sessions", systemImage: "list.bullet.rectangle", value: AppTab.sessions) {
                    NavigationStack(path: $router.sessionsPath) {
                        SessionsScreen()
                    }
                }
                Tab("Jobs", systemImage: "clock.badge.checkmark", value: AppTab.jobs) {
                    NavigationStack(path: $router.jobsPath) {
                        JobsScreen()
                    }
                }
                Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                    NavigationStack(path: $router.settingsPath) {
                        SettingsScreen()
                    }
                }
            }
            .sheet(isPresented: $router.showsApprovals) {
                NavigationStack {
                    ApprovalsScreen()
                }
            }
        } else {
            OnboardingFlow()
        }
    }
}
