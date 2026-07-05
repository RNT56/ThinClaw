// swift-tools-version: 6.2

// ThinClawTransport — SSE parsing, event decoding, and reconnect policy for
// the gateway event stream (`/api/chat/events`).
//
// Dependency direction: Transport -> Core (and nothing else). ThinClawCore is
// the dependency-free leaf that owns `AgentEvent`; Transport turns bytes into
// those events. Pure logic, no UI imports, so it declares macOS as well and
// tests run with plain `swift test` on a Mac host — no simulator, no network.
import PackageDescription

let package = Package(
    name: "ThinClawTransport",
    platforms: [
        .macOS(.v14),
        .iOS(.v26),
    ],
    products: [
        .library(name: "ThinClawTransport", targets: ["ThinClawTransport"])
    ],
    dependencies: [
        .package(path: "../ThinClawCore")
    ],
    targets: [
        .target(
            name: "ThinClawTransport",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore")
            ]
        ),
        .testTarget(
            name: "ThinClawTransportTests",
            dependencies: ["ThinClawTransport"],
            resources: [
                .copy("Fixtures")
            ]
        ),
    ]
)
