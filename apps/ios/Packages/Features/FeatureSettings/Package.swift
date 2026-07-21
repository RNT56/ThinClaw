// swift-tools-version: 6.2

// FeatureSettings — one surface of the ThinClaw iOS app. Feature packages expose a root
// SwiftUI view plus a @MainActor @Observable store; effectful dependencies
// arrive through initializer injection from the app composition root.
// iOS-only; compiled via the Tuist-generated project.
import PackageDescription

let package = Package(
    name: "FeatureSettings",
    platforms: [
        .iOS(.v18)
    ],
    products: [
        .library(name: "FeatureSettings", targets: ["FeatureSettings"])
    ],
    dependencies: [
        .package(path: "../../ThinClawCore"),
        .package(path: "../../ThinClawDesign"),
        .package(path: "../../ThinClawAuth"),
        .package(path: "../../ThinClawAPI"),
        .package(path: "../../ThinClawTransport"),
    ],
    targets: [
        .target(
            name: "FeatureSettings",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
                .product(name: "ThinClawAuth", package: "ThinClawAuth"),
                .product(name: "ThinClawAPI", package: "ThinClawAPI"),
                .product(name: "ThinClawTransport", package: "ThinClawTransport"),
            ]
        ),
        .testTarget(
            name: "FeatureSettingsTests",
            dependencies: [
                "FeatureSettings",
                .product(name: "ThinClawCore", package: "ThinClawCore"),
            ]
        ),
    ]
)
