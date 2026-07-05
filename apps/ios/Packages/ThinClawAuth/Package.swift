// swift-tools-version: 6.2

// ThinClawAuth — device pairing, credential storage, and gateway discovery.
//
// Pure-logic pieces (pairing payload parsing, credential model, in-memory
// keychain) are macOS-testable. Platform-coupled pieces (SecItem keychain,
// NWBrowser Bonjour discovery) sit behind `#if canImport(...)` and compile on
// every Apple platform, but only their protocol seams are unit-tested here.
import PackageDescription

let package = Package(
    name: "ThinClawAuth",
    platforms: [
        .macOS(.v14),
        .iOS(.v26),
        .watchOS(.v26),
    ],
    products: [
        .library(name: "ThinClawAuth", targets: ["ThinClawAuth"])
    ],
    targets: [
        .target(name: "ThinClawAuth"),
        .testTarget(
            name: "ThinClawAuthTests",
            dependencies: ["ThinClawAuth"]
        ),
    ]
)
