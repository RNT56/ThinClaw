// swift-tools-version: 6.2

// FeatureApprovals — one surface of the ThinClaw iOS app. Feature packages expose a root
// SwiftUI view plus a @MainActor @Observable store; effectful dependencies
// arrive through initializer injection from the app composition root.
// iOS-only; compiled via the Tuist-generated project.
import PackageDescription

let package = Package(
    name: "FeatureApprovals",
    platforms: [
        .iOS(.v18)
    ],
    products: [
        .library(name: "FeatureApprovals", targets: ["FeatureApprovals"])
    ],
    dependencies: [
        .package(path: "../../ThinClawCore"),
        .package(path: "../../ThinClawDesign"),
        .package(path: "../../ThinClawTransport"),
    ],
    targets: [
        .target(
            name: "FeatureApprovals",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
                .product(name: "ThinClawTransport", package: "ThinClawTransport"),
            ]
        ),
        .testTarget(
            name: "FeatureApprovalsTests",
            dependencies: [
                "FeatureApprovals",
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
            ]
        ),
    ]
)
