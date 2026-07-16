// swift-tools-version: 6.2

// ThinClawCore — pure domain types shared by every ThinClaw Apple target.
//
// Platform note: this package is pure logic (Foundation only, no UI imports)
// and deliberately declares BOTH macOS and iOS so its tests run with plain
// `swift test` on a Mac host, without an iOS simulator or Tuist.
//
// The one dependency, ThinClawSnapshotKit, is itself a Foundation-only leaf
// (Codable snapshot types + App Group file store) so the whole graph stays
// macOS-testable. Core owns SnapshotPublisher — the pure mapping from live
// domain state to those snapshot files — so it must produce SnapshotKit types.
import PackageDescription

let package = Package(
    name: "ThinClawCore",
    platforms: [
        .macOS(.v14),
        .iOS(.v18),
    ],
    products: [
        .library(name: "ThinClawCore", targets: ["ThinClawCore"])
    ],
    dependencies: [
        .package(path: "../ThinClawSnapshotKit")
    ],
    targets: [
        .target(
            name: "ThinClawCore",
            dependencies: ["ThinClawSnapshotKit"]
        ),
        .testTarget(
            name: "ThinClawCoreTests",
            dependencies: ["ThinClawCore"]
        ),
    ]
)
