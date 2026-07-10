// swift-tools-version: 6.2

// ThinClawAPI — typed gateway client.
//
// R0 ships the hand-written shell (endpoint description, token injection,
// error taxonomy). The generated REST client arrives when
// `scripts/generate-api.sh` runs Apple's swift-openapi-generator against the
// committed spec (`clients/openapi/thinclaw-gateway.openapi.json`) and the
// output is committed under `Sources/ThinClawAPI/Generated/`. Generated code
// is committed — iOS CI never needs the Rust toolchain (see apps/ios/README).
// Generation is script-driven, not build-plugin driven, so this target has no
// generator plugin — only the runtime + URLSession transport it links against.
//
// Pure logic, no UI imports; macOS platform is declared so `swift test`
// runs on a Mac host.
import PackageDescription

let package = Package(
    name: "ThinClawAPI",
    platforms: [
        .macOS(.v14),
        .iOS(.v18),
        // watchOS is declared so the watch-side direct-HTTP fallback route
        // (ThinClawWatchBridge `WatchGatewayProxy`, D-K4) can reach the gateway
        // with the generated REST client when relay is unavailable. Apple's
        // openapi-runtime/urlsession both support watchOS.
        .watchOS(.v11),
    ],
    products: [
        .library(name: "ThinClawAPI", targets: ["ThinClawAPI"])
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-openapi-runtime", from: "1.8.0"),
        .package(url: "https://github.com/apple/swift-openapi-urlsession", from: "1.1.0"),
        .package(url: "https://github.com/apple/swift-http-types", from: "1.4.0"),
    ],
    targets: [
        .target(
            name: "ThinClawAPI",
            dependencies: [
                .product(name: "OpenAPIRuntime", package: "swift-openapi-runtime"),
                .product(name: "OpenAPIURLSession", package: "swift-openapi-urlsession"),
                .product(name: "HTTPTypes", package: "swift-http-types"),
            ],
            // Docs that live next to committed generated sources — not build inputs.
            exclude: ["Generated/README.md"]
        ),
        .testTarget(
            name: "ThinClawAPITests",
            dependencies: ["ThinClawAPI"]
        ),
    ]
)
