// swift-tools-version: 6.2

// ThinClawPersistence — local transcript cache and offline send-outbox.
//
// R0 shipped the storage protocol plus an in-memory implementation so feature
// code and tests had a seam. M1 adds the production GRDB-backed store
// (WAL `DatabasePool`, app-process-only; extensions read `ThinClawSnapshotKit`
// files instead of touching this database). GRDB is pinned to an exact 7.x
// release so a resolver drift can never silently change the on-disk format
// under us.
//
// The store is a cache: the gateway owns history, so schema resets are
// always recoverable by re-syncing.
import PackageDescription

let package = Package(
    name: "ThinClawPersistence",
    platforms: [
        .macOS(.v14),
        .iOS(.v18),
    ],
    products: [
        .library(name: "ThinClawPersistence", targets: ["ThinClawPersistence"])
    ],
    dependencies: [
        .package(path: "../ThinClawCore"),
        .package(url: "https://github.com/groue/GRDB.swift.git", exact: "7.11.1"),
    ],
    targets: [
        .target(
            name: "ThinClawPersistence",
            dependencies: [
                .product(name: "ThinClawCore", package: "ThinClawCore"),
                .product(name: "GRDB", package: "GRDB.swift"),
            ]
        ),
        .testTarget(
            name: "ThinClawPersistenceTests",
            dependencies: ["ThinClawPersistence"]
        ),
    ]
)
