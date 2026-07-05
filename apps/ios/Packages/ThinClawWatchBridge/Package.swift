// swift-tools-version: 6.2

// ThinClawWatchBridge — WatchConnectivity relay between iPhone and watch.
//
// Relay-first by design: there is no Tailscale client on watchOS, so the
// paired iPhone is the reliable route to a tailnet gateway. The watch
// attaches its OWN reduced-scope token to relayed requests (the phone
// forwards opaquely) so the gateway attributes and revokes it independently
// (docs/MOBILE_SECURITY.md, D-K4). Direct HTTP from the watch remains a
// fallback for LAN / public-HTTPS gateways.
//
// Host (iOS) and client (watchOS) halves live in one package; each target
// compiles only the half its platform supports via canImport(WatchConnectivity)
// availability and target-membership in the Tuist project.
import PackageDescription

let package = Package(
    name: "ThinClawWatchBridge",
    platforms: [
        .iOS(.v26),
        .watchOS(.v26),
    ],
    products: [
        .library(name: "ThinClawWatchBridge", targets: ["ThinClawWatchBridge"])
    ],
    dependencies: [
        .package(path: "../ThinClawCore"),
        .package(path: "../ThinClawSnapshotKit"),
    ],
    targets: [
        .target(
            name: "ThinClawWatchBridge",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawSnapshotKit", package: "ThinClawSnapshotKit"),
            ]
        )
    ]
)
