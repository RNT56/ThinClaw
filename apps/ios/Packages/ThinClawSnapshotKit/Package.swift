// swift-tools-version: 6.2

// ThinClawSnapshotKit — Codable snapshot types + App Group file store shared
// between the iOS app, its widget extension, and (via WatchConnectivity
// mirroring) the watch app and complications.
//
// Foundation-only by design: WidgetKit timeline providers read these files in
// very tight memory budgets, and the package must stay macOS-testable so
// `swift test` runs on a Mac host without a simulator.
import PackageDescription

let package = Package(
    name: "ThinClawSnapshotKit",
    platforms: [
        .macOS(.v14),
        .iOS(.v18),
        .watchOS(.v11),
    ],
    products: [
        .library(name: "ThinClawSnapshotKit", targets: ["ThinClawSnapshotKit"])
    ],
    targets: [
        .target(name: "ThinClawSnapshotKit"),
        .testTarget(
            name: "ThinClawSnapshotKitTests",
            dependencies: ["ThinClawSnapshotKit"]
        ),
    ]
)
