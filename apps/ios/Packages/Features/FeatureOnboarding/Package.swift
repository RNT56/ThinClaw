// swift-tools-version: 6.2

// FeatureOnboarding — one surface of the ThinClaw iOS app. Feature packages expose a root
// SwiftUI view plus a @MainActor @Observable store; effectful dependencies
// arrive through initializer injection from the app composition root.
// iOS-only; compiled via the Tuist-generated project.
import PackageDescription

let package = Package(
    name: "FeatureOnboarding",
    platforms: [
        .iOS(.v26)
    ],
    products: [
        .library(name: "FeatureOnboarding", targets: ["FeatureOnboarding"])
    ],
    dependencies: [
        .package(path: "../../ThinClawCore"),
        .package(path: "../../ThinClawDesign"),
        .package(path: "../../ThinClawAuth"), .package(path: "../../ThinClawAPI"),
    ],
    targets: [
        .target(
            name: "FeatureOnboarding",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
                .product(name: "ThinClawAuth", package: "ThinClawAuth"),
                .product(name: "ThinClawAPI", package: "ThinClawAPI"),
            ]
        )
    ]
)
