// swift-tools-version: 6.2

// ThinClawPersistence — local transcript cache and offline send-outbox.
//
// R0 ships the storage protocol plus an in-memory implementation so feature
// code and tests have a seam today. The GRDB-backed store (WAL DatabasePool,
// app-process-only; extensions read ThinClawSnapshotKit files instead)
// lands at milestone M1 — GRDB is deliberately NOT a dependency yet so the
// R0 scaffold builds fully offline.
//
// The store is a cache: the gateway owns history, so schema resets are
// always recoverable by re-syncing.
import PackageDescription

let package = Package(
    name: "ThinClawPersistence",
    platforms: [
        .macOS(.v14),
        .iOS(.v26),
    ],
    products: [
        .library(name: "ThinClawPersistence", targets: ["ThinClawPersistence"])
    ],
    dependencies: [
        .package(path: "../ThinClawCore")
    ],
    targets: [
        .target(
            name: "ThinClawPersistence",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore")
            ]
        ),
        .testTarget(
            name: "ThinClawPersistenceTests",
            dependencies: ["ThinClawPersistence"]
        ),
    ]
)
