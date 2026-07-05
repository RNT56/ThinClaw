// swift-tools-version: 6.2

// ThinClawCore — pure domain types shared by every ThinClaw Apple target.
//
// Platform note: this package is pure logic (Foundation only, no UI imports)
// and deliberately declares BOTH macOS and iOS so its tests run with plain
// `swift test` on a Mac host, without an iOS simulator or Tuist.
import PackageDescription

let package = Package(
    name: "ThinClawCore",
    platforms: [
        .macOS(.v14),
        .iOS(.v26),
    ],
    products: [
        .library(name: "ThinClawCore", targets: ["ThinClawCore"])
    ],
    targets: [
        .target(name: "ThinClawCore"),
        .testTarget(
            name: "ThinClawCoreTests",
            dependencies: ["ThinClawCore"]
        ),
    ]
)
