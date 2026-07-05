// swift-tools-version: 6.2

// ThinClawWidgetKitShared — timeline providers and the interactive
// AppIntents (approve/deny/quick-ask) shared by the widget extension, the
// app (for App Shortcuts / Siri registration), and deep-link routing.
//
// The widget process reads SnapshotKit files and calls the gateway REST API
// directly with the shared-Keychain device token; it never imports the
// streaming stack (ThinClawTransport) — short-lived extension processes
// stay small and fast.
import PackageDescription

let package = Package(
    name: "ThinClawWidgetKitShared",
    platforms: [
        .iOS(.v26)
    ],
    products: [
        .library(name: "ThinClawWidgetKitShared", targets: ["ThinClawWidgetKitShared"])
    ],
    dependencies: [
        .package(path: "../ThinClawSnapshotKit"),
        .package(path: "../ThinClawAuth"),
        .package(path: "../ThinClawAPI"),
        .package(path: "../ThinClawDesign"),
    ],
    targets: [
        .target(
            name: "ThinClawWidgetKitShared",
            dependencies: [
                .product(name: "ThinClawSnapshotKit", package: "ThinClawSnapshotKit"),
                .product(name: "ThinClawAuth", package: "ThinClawAuth"),
                .product(name: "ThinClawAPI", package: "ThinClawAPI"),
                .product(name: "ThinClawDesign", package: "ThinClawDesign"),
            ]
        )
    ]
)
