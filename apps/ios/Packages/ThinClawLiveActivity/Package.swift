// swift-tools-version: 6.2

// ThinClawLiveActivity — the Live Activity manager for an agent run (M3).
//
// Two layers, split so the decision logic is testable on a Mac host without a
// simulator:
//   - `RunTracker`: a pure, Foundation-only reducer that turns a sequence of
//     run events (start signals, progress, completion, and *inbound push
//     revisions*) into `RunAction`s with a monotonically increasing revision.
//     This is what `swift test` exercises — no ActivityKit, no iOS.
//   - `LiveActivityManager` (iOS-only, behind `canImport(ActivityKit)`): the
//     `@MainActor` object that observes a `GatewaySession`, drives the reducer,
//     and performs the real `Activity.request`/`.update`/`.end` and push-token
//     registration through the `ActivityController`/`LiveActivityRegistrar`
//     protocols. Tests fake those protocols.
//
// Depends on ThinClawCore (ThreadID/AgentEvent), ThinClawSnapshotKit
// (AgentRunAttributes, iOS-only), ThinClawAPI (the generated client for token
// registration), and ThinClawTransport (GatewaySession) only on iOS.
import PackageDescription

let package = Package(
    name: "ThinClawLiveActivity",
    platforms: [
        .macOS(.v14),
        .iOS(.v18),
    ],
    products: [
        .library(name: "ThinClawLiveActivity", targets: ["ThinClawLiveActivity"])
    ],
    dependencies: [
        .package(path: "../ThinClawCore"),
        .package(path: "../ThinClawSnapshotKit"),
        .package(path: "../ThinClawAPI"),
        .package(path: "../ThinClawTransport"),
    ],
    targets: [
        .target(
            name: "ThinClawLiveActivity",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawSnapshotKit", package: "ThinClawSnapshotKit"),
                .product(name: "ThinClawAPI", package: "ThinClawAPI"),
                .product(name: "ThinClawTransport", package: "ThinClawTransport"),
            ]
        ),
        .testTarget(
            name: "ThinClawLiveActivityTests",
            dependencies: ["ThinClawLiveActivity"]
        ),
    ]
)
