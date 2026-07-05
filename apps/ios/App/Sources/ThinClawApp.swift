import FeatureApprovals
import FeatureChat
import FeatureJobs
import FeatureOnboarding
import FeatureSessions
import FeatureSettings
import SwiftUI
import ThinClawCore
import ThinClawDesign

@main
struct ThinClawApp: App {
    @State private var dependencies = AppDependencies()
    @State private var router = AppRouter()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(dependencies)
                .environment(router)
                .onOpenURL { url in
                    router.handle(deepLink: url)
                }
        }
        .onChange(of: scenePhase) { _, phase in
            // Start the live event stream only while paired and foregrounded;
            // tear it down when backgrounded so the app is not holding a socket
            // (and battery) open in the background.
            switch phase {
            case .active:
                Task { await dependencies.startSessionIfPaired() }
            case .background:
                Task { await dependencies.stopSession() }
            default:
                break
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
                        ChatTab()
                    }
                }
                Tab("Sessions", systemImage: "list.bullet.rectangle", value: AppTab.sessions) {
                    NavigationStack(path: $router.sessionsPath) {
                        SessionsTab()
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
            OnboardingFlow(store: dependencies.makeOnboardingStore())
        }
    }
}

/// The Chat tab resolves which thread to show — the router's selected thread
/// (from a Sessions tap or a deep link) or, absent a selection, the most-recent
/// cached thread — and builds a fresh ``ChatStore`` for it. Re-identifying the
/// view by thread id tears down and rebuilds the store when the thread changes,
/// so each thread gets its own subscription and reducer.
struct ChatTab: View {
    @Environment(AppDependencies.self) private var dependencies
    @Environment(AppRouter.self) private var router

    @State private var resolvedThread: ThreadID?

    var body: some View {
        Group {
            if let thread = router.selectedThread ?? resolvedThread,
                let store = dependencies.makeChatStore(thread: thread)
            {
                ChatScreen(store: store)
                    .id(thread)
            } else {
                ContentUnavailableView(
                    "No conversation yet",
                    systemImage: "bubble.left.and.text.bubble.right",
                    description: Text("Start a message or pick a session."))
            }
        }
        .task {
            // Resolve a default thread once, so the Chat tab opens something
            // even before the user visits Sessions.
            if router.selectedThread == nil, resolvedThread == nil {
                resolvedThread = await dependencies.defaultThread()
            }
        }
    }
}

/// The Sessions tab builds its store from the dependency graph and routes a row
/// tap into the Chat tab.
struct SessionsTab: View {
    @Environment(AppDependencies.self) private var dependencies
    @Environment(AppRouter.self) private var router

    var body: some View {
        if let store = dependencies.makeSessionsStore() {
            SessionsScreen(store: store) { threadID in
                router.openThread(threadID)
            }
        } else {
            ContentUnavailableView(
                "Not connected", systemImage: "wifi.slash",
                description: Text("Reconnect to load your sessions."))
        }
    }
}
