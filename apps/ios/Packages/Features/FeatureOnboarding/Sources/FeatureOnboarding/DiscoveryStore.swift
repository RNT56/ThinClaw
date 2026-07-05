import Foundation
import Observation
import ThinClawAuth

/// A source of ``DiscoveredGateway`` sets for onboarding. Abstracted so
/// ``DiscoveryStore`` can be exercised with a scripted stream in tests without a
/// live `NWBrowser`/network. `ThinClawAuth.BonjourBrowser` is the production
/// conformer.
public protocol GatewayDiscovering: Sendable {
    /// Each element is the *complete current set* of discovered gateways (not a
    /// delta). The stream ends when browsing stops or the browser fails.
    func gatewaySets() -> AsyncStream<[DiscoveredGateway]>
    /// Stop browsing and release the underlying browser.
    func stop()
}

#if canImport(Network)
    extension BonjourBrowser: GatewayDiscovering {}
#endif

/// Drives the "Discover on this network" affordance in onboarding (milestone
/// B3). Browses `_thinclaw._tcp` and republishes the current set of candidate
/// gateways for the welcome UI.
///
/// **Locator only.** Discovery just *finds* candidate endpoints. Selecting one
/// pre-fills the pairing URL, but pairing still requires the QR secret (or a
/// typed short code) and the connection still verifies the pinned SPKI +
/// instance id before any credential is sent (docs/MOBILE_SECURITY.md D-X3 /
/// T11). Nothing here authenticates a gateway; ``DiscoveryStore`` never issues
/// a network request to a discovered endpoint.
@MainActor
@Observable
public final class DiscoveryStore {
    /// Whether the user has opened the discovery affordance and browsing is
    /// live. Local-network permission is only prompted once browsing starts.
    public private(set) var isBrowsing = false

    /// The current set of discovered gateways, sorted by display name. Replaced
    /// wholesale on every emission, so a gateway going offline simply drops.
    public private(set) var gateways: [DiscoveredGateway] = []

    private let browser: any GatewayDiscovering
    private var browseTask: Task<Void, Never>?

    public init(browser: any GatewayDiscovering) {
        self.browser = browser
    }

    #if canImport(Network)
        /// Production convenience: browse with a real `BonjourBrowser`.
        public convenience init() {
            self.init(browser: BonjourBrowser())
        }
    #endif

    /// Begin browsing (prompts local-network permission the first time). Safe to
    /// call repeatedly; a second call while already browsing is a no-op.
    public func start() {
        guard browseTask == nil else { return }
        isBrowsing = true
        gateways = []
        let browser = browser
        browseTask = Task { [weak self] in
            for await set in browser.gatewaySets() {
                if Task.isCancelled { break }
                self?.gateways = set
            }
            await self?.markStopped()
        }
    }

    /// Stop browsing and clear the candidate list.
    public func stop() {
        browseTask?.cancel()
        browseTask = nil
        browser.stop()
        isBrowsing = false
        gateways = []
    }

    private func markStopped() {
        browseTask = nil
        isBrowsing = false
    }
}
