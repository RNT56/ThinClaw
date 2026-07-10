// swift-tools-version: 6.2

// ThinClawDesign — the Liquid Glass design system: tokens plus the reusable
// chat/status components every feature package composes.
//
// iOS-only (SwiftUI + glassEffect APIs); compiled through the Tuist-generated
// project / xcodebuild with an iOS destination, not `swift test` on a Mac
// host. Keep this package dependency-free so widgets and the watch app can
// import it without dragging in transport or persistence.
import PackageDescription

let package = Package(
    name: "ThinClawDesign",
    platforms: [
        .iOS(.v18),
        .watchOS(.v11),
    ],
    products: [
        .library(name: "ThinClawDesign", targets: ["ThinClawDesign"])
    ],
    targets: [
        .target(name: "ThinClawDesign")
    ]
)
