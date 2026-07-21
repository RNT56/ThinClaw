import Foundation

/// Central file-protection policy for app and App Group artifacts.
public struct DataProtectionPolicy: Sendable, Equatable {
    public static let enhancedPreferenceKey =
        "com.thinclaw.ios.settings.enhancedProtection"

    public var enhanced: Bool

    public init(enhanced: Bool = false) {
        self.enhanced = enhanced
    }

    @discardableResult
    public func apply(to url: URL) -> Bool {
        #if os(iOS) || os(watchOS)
            let protection: FileProtectionType =
                enhanced ? .complete : .completeUntilFirstUserAuthentication
            do {
                try FileManager.default.setAttributes(
                    [.protectionKey: protection],
                    ofItemAtPath: url.path)
                return true
            } catch {
                return false
            }
        #else
            return true
        #endif
    }
}
