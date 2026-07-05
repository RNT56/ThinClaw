// swift-tools-version: 6.2

// ThinClawTransport — SSE parsing, event decoding, reconnect policy, the live
// event stream, and the gateway session for the mobile client.
//
// Dependency direction: Transport -> Core (domain models) and Transport ->
// ThinClawAPI (generated REST client). Both are lower layers: ThinClawCore is
// the pure leaf that owns `AgentEvent` and the domain result types; ThinClawAPI
// owns the typed REST surface. Transport is where bytes become events and where
// the REST client + event stream are composed into a session — so it must sit
// above both. Neither Core nor API depends back on Transport, so this stays
// acyclic. Pure logic, no UI imports, so it declares macOS as well and tests
// run with plain `swift test` on a Mac host — no simulator, no network.
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
        .package(path: "../ThinClawCore"),
        .package(path: "../ThinClawAPI"),
    ],
    targets: [
        .target(
            name: "ThinClawTransport",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawAPI", package: "ThinClawAPI"),
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
