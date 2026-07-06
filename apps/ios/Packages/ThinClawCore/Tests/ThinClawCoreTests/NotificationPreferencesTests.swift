import Foundation
import Testing

@testable import ThinClawCore

// MARK: - Model invariants

@Test func defaultsAreWhenUnlockedEverywhere() {
    let prefs = NotificationPreferences.default
    for category in NotificationCategory.allCases {
        #expect(prefs.mode(for: category) == .whenUnlocked)
    }
}

@Test func appOnlyIsApprovalOnly() {
    // appOnly is a valid approval mode…
    #expect(NotificationPreferences.allowedModes(for: .approval).contains(.appOnly))
    // …but not offered for message/job.
    #expect(!NotificationPreferences.allowedModes(for: .message).contains(.appOnly))
    #expect(!NotificationPreferences.allowedModes(for: .job).contains(.appOnly))
}

@Test func appOnlyOnNonApprovalCoercesToNever() {
    let prefs = NotificationPreferences.default
        .setting(.appOnly, for: .message)
        .setting(.appOnly, for: .job)
    #expect(prefs.message == .never)
    #expect(prefs.job == .never)

    // On approvals it is preserved.
    let approvals = NotificationPreferences.default.setting(.appOnly, for: .approval)
    #expect(approvals.approval == .appOnly)
}

// MARK: - Rewrite gating (the NSE decision)

@Test func rewriteGatingByMode() {
    #expect(PreviewMode.always.allowsRewrite(deviceUnlocked: false) == true)
    #expect(PreviewMode.always.allowsRewrite(deviceUnlocked: nil) == true)

    #expect(PreviewMode.never.allowsRewrite(deviceUnlocked: true) == false)
    #expect(PreviewMode.appOnly.allowsRewrite(deviceUnlocked: true) == false)

    #expect(PreviewMode.whenUnlocked.allowsRewrite(deviceUnlocked: true) == true)
    #expect(PreviewMode.whenUnlocked.allowsRewrite(deviceUnlocked: false) == false)
    // Unknown lock state fails closed.
    #expect(PreviewMode.whenUnlocked.allowsRewrite(deviceUnlocked: nil) == false)
}

// MARK: - Persistence round-trip

@Test func persistenceRoundTrip() {
    let kv = InMemoryKeyValueStore()
    let store = NotificationPreferencesStore(store: kv)

    let saved = NotificationPreferences(message: .never, approval: .appOnly, job: .always)
    store.save(saved)

    let loaded = store.load()
    #expect(loaded.message == .never)
    #expect(loaded.approval == .appOnly)
    #expect(loaded.job == .always)

    // The NSE reads a single category key directly.
    #expect(store.mode(for: .approval) == .appOnly)
    #expect(kv.string(forKey: NotificationPreferencesStore.key(for: .approval)) == "appOnly")
}

@Test func unsetKeysFallBackToDefault() {
    let store = NotificationPreferencesStore(store: InMemoryKeyValueStore())
    // Nothing persisted yet → defaults.
    #expect(store.load() == .default)
    #expect(store.mode(for: .message) == .whenUnlocked)
}

@Test func invalidPersistedValueFallsBackToDefault() {
    // A garbage value, or a value invalid for the category (appOnly on message),
    // both collapse to the category default rather than crashing or leaking a
    // wrong mode into the NSE.
    let kv = InMemoryKeyValueStore([
        NotificationPreferencesStore.key(for: .message): "appOnly",
        NotificationPreferencesStore.key(for: .job): "nonsense",
    ])
    let store = NotificationPreferencesStore(store: kv)
    #expect(store.mode(for: .message) == .whenUnlocked)
    #expect(store.mode(for: .job) == .whenUnlocked)
}

@Test func previewModeWireValuesAreStable() {
    // The NSE relies on these exact raw strings; pin them so a rename can't
    // silently break the cross-process contract.
    #expect(PreviewMode.always.rawValue == "always")
    #expect(PreviewMode.whenUnlocked.rawValue == "whenUnlocked")
    #expect(PreviewMode.never.rawValue == "never")
    #expect(PreviewMode.appOnly.rawValue == "appOnly")
}
