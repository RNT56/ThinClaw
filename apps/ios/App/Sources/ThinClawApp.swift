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
    @State private var dependencies: AppDependencies
    @State private var router: AppRouter
    @State private var pushCoordinator: PushCoordinator
    @Environment(\.scenePhase) private var scenePhase

    #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
        // Owns the watch companion relay host (M4, D-K4). Thin app-side hook:
        // activation while paired + deprovision on unpair; all testable logic
        // lives in ThinClawWatchBridge.
        @State private var watchProvisioning = WatchProvisioning()
    #endif

    #if canImport(UIKit)
        @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #endif

    init() {
        // Build the graph, then the notification coordinator over it. `@State`
        // wrappers are initialized directly (not `= …` defaults) because the
        // coordinator needs the same `dependencies`/`router` instances the rest
        // of the app observes.
        let dependencies = AppDependencies()
        let router = AppRouter()
        _dependencies = State(initialValue: dependencies)
        _router = State(initialValue: router)
        _pushCoordinator = State(
            initialValue: PushCoordinator(dependencies: dependencies, router: router))
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(dependencies)
                .environment(router)
                .onOpenURL { url in
                    router.handle(deepLink: url)
                }
                .task {
                    // Install the notification-center delegate + categories and
                    // hand the delegate its dependencies once, at launch.
                    pushCoordinator.configure()
                    #if canImport(UIKit)
                        appDelegate.dependencies = dependencies
                        appDelegate.pushCoordinator = pushCoordinator
                    #endif
                }
        }
        .onChange(of: scenePhase) { _, phase in
            // Start the live event stream only while paired and foregrounded;
            // tear it down when backgrounded so the app is not holding a socket
            // (and battery) open in the background.
            switch phase {
            case .active:
                Task { await dependencies.startSessionIfPaired() }
                #if canImport(UIKit)
                    // Register for APNs each foreground while paired; the OS
                    // dedupes and returns a token to the delegate. Content-free
                    // pushes (D-N1) then flow to PushCoordinator.
                    if dependencies.isPaired {
                        appDelegate.requestPushAuthorizationAndRegister()
                    }
                #endif
                #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
                    // Activate the watch relay host while paired so a paired watch
                    // provisions its own companion token and can relay approvals
                    // through the phone (M4, D-K4). Idempotent.
                    if dependencies.isPaired {
                        watchProvisioning.activateIfPaired()
                    }
                #endif
            case .background:
                Task { await dependencies.stopSession() }
                // Arm the periodic BGAppRefresh safety net so widgets keep
                // updating even if no silent push arrives while backgrounded.
                #if canImport(UIKit)
                    BackgroundRefresh.scheduleAppRefresh()
                #endif
            default:
                break
            }
        }
        #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
            .onChange(of: dependencies.isPaired) { _, paired in
                // On unpair, best-effort revoke the watch companion and drop the
                // relay host (the parent-revoke cascade also covers it). On a
                // fresh pair, activate so the watch can provision.
                if paired {
                    watchProvisioning.activateIfPaired()
                } else {
                    Task { await watchProvisioning.deprovisionAndTearDown() }
                }
            }
        #endif
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
                    if let store = dependencies.makeApprovalsStore() {
                        ApprovalsScreen(store: store)
                    } else {
                        ContentUnavailableView(
                            "No pending approvals",
                            systemImage: "checkmark.shield")
                    }
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
            startLiveActivityForActiveThread()
        }
        .onChange(of: router.selectedThread) { _, _ in
            startLiveActivityForActiveThread()
        }
    }

    /// Point the Live Activity manager at the Chat tab's active thread so an
    /// agent run on it drives the Dynamic Island / lock-screen activity. The
    /// manager owns at most one activity per thread and is idempotent per
    /// thread, so re-calling on every thread change is safe. The thread id is
    /// used as a best-effort activity title until a richer title is threaded
    /// through the resolver.
    private func startLiveActivityForActiveThread() {
        #if canImport(ActivityKit)
            guard let thread = router.selectedThread ?? resolvedThread else { return }
            dependencies.startLiveActivity(for: thread, title: thread.rawValue)
        #endif
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
