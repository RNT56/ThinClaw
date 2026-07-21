import FeatureApprovals
import FeatureChat
import FeatureJobs
import FeatureOnboarding
import FeatureSessions
import FeatureSettings
import SwiftUI
import ThinClawAuth
import ThinClawCore
import ThinClawDesign
import ThinClawWidgetKitShared

@main
struct ThinClawApp: App {
    @State private var coordinator = AppCoordinator()
    @Environment(\.scenePhase) private var scenePhase

    #if canImport(UIKit)
        @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #endif

    var body: some Scene {
        WindowGroup {
            @Bindable var coordinator = coordinator
            RootView()
                .environment(coordinator.dependencies)
                .environment(coordinator.router)
                .uiTestAppearance()
                // App-switcher / snapshot redaction: always cover the window
                // while the scene is inactive/backgrounded so the multitasking
                // snapshot never leaks transcript content (M5,
                // docs/MOBILE_SECURITY.md).
                .privacyOverlay()
                .onOpenURL { coordinator.handleOpenURL($0) }
                .task {
                    #if canImport(UIKit)
                        coordinator.configure(appDelegate: appDelegate)
                    #else
                        coordinator.configure(appDelegate: nil)
                    #endif
                }
                .alert(
                    "Replace the paired gateway?",
                    isPresented: Binding(
                        get: { coordinator.pendingGatewayReplacementURL != nil },
                        set: { if !$0 { coordinator.pendingGatewayReplacementURL = nil } })
                ) {
                    Button("Cancel", role: .cancel) {
                        coordinator.pendingGatewayReplacementURL = nil
                    }
                    Button("Replace gateway", role: .destructive) {
                        Task { await coordinator.replaceGateway() }
                    }
                } message: {
                    Text(
                        "This revokes the current device, clears its local data, and starts pairing with the new gateway."
                    )
                }
        }
        .onChange(of: scenePhase) { _, phase in
            #if canImport(UIKit)
                coordinator.sceneDidChange(to: phase, appDelegate: appDelegate)
            #else
                coordinator.sceneDidChange(to: phase, appDelegate: nil)
            #endif
        }
        .onChange(of: coordinator.dependencies.isPaired) { _, paired in
            coordinator.pairingStateDidChange(paired)
        }
    }
}

/// Deterministic visual fixtures for XCUITest. Simulator preference arguments
/// are not reliable across runtime versions, so the test build applies the
/// requested SwiftUI environment directly. Release builds always return the
/// untouched content.
private struct UITestAppearanceModifier: ViewModifier {
    @ViewBuilder
    func body(content: Content) -> some View {
        #if DEBUG
            let arguments = ProcessInfo.processInfo.arguments
            if arguments.contains("--uitesting-dark-accessibility") {
                content
                    .preferredColorScheme(.dark)
                    .dynamicTypeSize(.accessibility5)
            } else if arguments.contains("--uitesting-light") {
                content.preferredColorScheme(.light)
            } else {
                content
            }
        #else
            content
        #endif
    }
}

extension View {
    fileprivate func uiTestAppearance() -> some View {
        modifier(UITestAppearanceModifier())
    }
}

/// Tab shell (Chat / Sessions / Approvals / Jobs / Settings). Onboarding replaces the shell
/// until a device credential exists.
struct RootView: View {
    @Environment(AppDependencies.self) private var dependencies
    @Environment(AppRouter.self) private var router

    var body: some View {
        if dependencies.isPaired {
            AuthenticatedShell()
        } else {
            OnboardingFlow(store: dependencies.makeOnboardingStore())
        }
    }
}

/// Compact tab navigation on iPhone and a persistent sidebar/detail shell on
/// regular-width iPad. Both presentations drive the same typed router.
private struct AuthenticatedShell: View {
    @Environment(AppDependencies.self) private var dependencies
    @Environment(AppRouter.self) private var router
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    var body: some View {
        @Bindable var router = router
        if horizontalSizeClass == .regular {
            NavigationSplitView {
                List(
                    selection: Binding<AppTab?>(
                        get: { router.selectedTab },
                        set: { if let tab = $0 { router.selectedTab = tab } }
                    )
                ) {
                    sidebarRow("Chat", icon: "bubble.left.and.text.bubble.right", tab: .chat)
                    sidebarRow("Sessions", icon: "list.bullet.rectangle", tab: .sessions)
                    sidebarRow("Approvals", icon: "checkmark.shield", tab: .approvals)
                    sidebarRow("Jobs", icon: "clock.badge.checkmark", tab: .jobs)
                    sidebarRow("Settings", icon: "gearshape", tab: .settings)
                }
                .navigationTitle("ThinClaw")
            } detail: {
                tabContent(router.selectedTab)
            }
        } else {
            TabView(selection: $router.selectedTab) {
                Tab("Chat", systemImage: "bubble.left.and.text.bubble.right", value: AppTab.chat) {
                    tabContent(.chat)
                }
                Tab("Sessions", systemImage: "list.bullet.rectangle", value: AppTab.sessions) {
                    tabContent(.sessions)
                }
                Tab("Approvals", systemImage: "checkmark.shield", value: AppTab.approvals) {
                    tabContent(.approvals)
                }
                .badge(dependencies.makeApprovalsStore()?.pending.count ?? 0)
                Tab("Jobs", systemImage: "clock.badge.checkmark", value: AppTab.jobs) {
                    tabContent(.jobs)
                }
                Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                    tabContent(.settings)
                }
            }
        }
    }

    private func sidebarRow(_ title: String, icon: String, tab: AppTab) -> some View {
        Label {
            HStack {
                Text(title)
                if tab == .approvals,
                    let count = dependencies.makeApprovalsStore()?.pending.count,
                    count > 0
                {
                    Spacer()
                    Text("\(count)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        } icon: {
            Image(systemName: icon)
        }
        .tag(tab)
    }

    @ViewBuilder
    private func tabContent(_ tab: AppTab) -> some View {
        @Bindable var router = router
        switch tab {
        case .chat:
            NavigationStack(path: $router.chatPath) { ChatTab() }
        case .sessions:
            NavigationStack(path: $router.sessionsPath) { SessionsTab() }
        case .approvals:
            NavigationStack(path: $router.approvalsPath) {
                if let store = dependencies.makeApprovalsStore() {
                    ApprovalsScreen(
                        store: store,
                        focusedRequestID: $router.focusedApprovalID)
                } else {
                    ContentUnavailableView(
                        "No pending approvals",
                        systemImage: "checkmark.shield")
                }
            }
        case .jobs:
            NavigationStack(path: $router.jobsPath) {
                JobsScreen(
                    store: { dependencies.makeJobsStore() },
                    selectedJobID: $router.focusedJobID)
            }
        case .settings:
            NavigationStack(path: $router.settingsPath) {
                if let store = dependencies.makeSettingsStore() {
                    SettingsScreen(store: store)
                } else {
                    ContentUnavailableView(
                        "Settings unavailable",
                        systemImage: "gearshape",
                        description: Text(
                            "Reconnect to your gateway to manage this device."))
                }
            }
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
                ChatScreen(store: store, approvalsStore: dependencies.makeApprovalsStore())
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
