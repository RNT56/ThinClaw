// swift-tools-version: 6.2

// ThinClawAPI — typed gateway client.
//
// R0 ships the hand-written shell (endpoint description, token injection,
// error taxonomy). The bulk of this package arrives when
// `scripts/generate-api.sh` runs Apple's swift-openapi-generator against the
// committed spec (`clients/openapi/thinclaw-gateway.openapi.json`) and the
// output is committed under `Sources/ThinClawAPI/Generated/`. Generated code
// is committed — iOS CI never needs the Rust toolchain (see apps/ios/README).
//
// Pure logic, no UI imports; macOS platform is declared so `swift test`
// runs on a Mac host.
import PackageDescription

let package = Package(
    name: "ThinClawAPI",
    platforms: [
        .macOS(.v14),
        .iOS(.v26),
    ],
    products: [
        .library(name: "ThinClawAPI", targets: ["ThinClawAPI"])
    ],
    targets: [
        .target(name: "ThinClawAPI"),
        .testTarget(
            name: "ThinClawAPITests",
            dependencies: ["ThinClawAPI"]
        ),
    ]
)
