// swift-tools-version: 6.2

// FeatureChat — one surface of the ThinClaw iOS app. Feature packages expose a root
// SwiftUI view plus a @MainActor @Observable store; effectful dependencies
// arrive through initializer injection from the app composition root.
// iOS-only; compiled via the Tuist-generated project.
import PackageDescription

let package = Package(
    name: "FeatureChat",
    platforms: [
        .iOS(.v26)
    ],
    products: [
        .library(name: "FeatureChat", targets: ["FeatureChat"])
    ],
    dependencies: [
        .package(path: "../../ThinClawCore"),
        .package(path: "../../ThinClawDesign"),
        .package(path: "../../ThinClawPersistence"),
    ],
    targets: [
        .target(
            name: "FeatureChat",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
                .product(name: "ThinClawPersistence", package: "ThinClawPersistence"),
            ]
        )
    ]
)
