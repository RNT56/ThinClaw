// swift-tools-version: 6.2

// FeatureJobs — one surface of the ThinClaw iOS app. Feature packages expose a root
// SwiftUI view plus a @MainActor @Observable store; effectful dependencies
// arrive through initializer injection from the app composition root.
// iOS-only; compiled via the Tuist-generated project.
import PackageDescription

let package = Package(
    name: "FeatureJobs",
    platforms: [
        .iOS(.v26)
    ],
    products: [
        .library(name: "FeatureJobs", targets: ["FeatureJobs"])
    ],
    dependencies: [
        .package(path: "../../ThinClawAPI"),
        .package(path: "../../ThinClawCore"),
        .package(path: "../../ThinClawDesign"),
    ],
    targets: [
        .target(
            name: "FeatureJobs",
            dependencies: [
                .product(name: "ThinClawAPI", package: "ThinClawAPI"),
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
            ]
        )
    ]
)
