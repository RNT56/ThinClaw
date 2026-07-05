// swift-tools-version: 6.2

// FeatureSessions — one surface of the ThinClaw iOS app. Feature packages expose a root
// SwiftUI view plus a @MainActor @Observable store; effectful dependencies
// arrive through initializer injection from the app composition root.
// iOS-only; compiled via the Tuist-generated project.
import PackageDescription

let package = Package(
    name: "FeatureSessions",
    platforms: [
        .iOS(.v26)
    ],
    products: [
        .library(name: "FeatureSessions", targets: ["FeatureSessions"])
    ],
    dependencies: [
        .package(path: "../../ThinClawCore"),
        .package(path: "../../ThinClawDesign"),
        .package(path: "../../ThinClawPersistence"),
    ],
    targets: [
        .target(
            name: "FeatureSessions",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
                .product(name: "ThinClawPersistence", package: "ThinClawPersistence"),
            ]
        )
    ]
)
