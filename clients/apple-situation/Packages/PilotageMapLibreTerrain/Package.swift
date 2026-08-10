// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "PilotageMapLibreTerrain",
    platforms: [
        .iOS(.v18),
    ],
    products: [
        .library(name: "MapLibre", targets: ["MapLibre"]),
    ],
    targets: [
        .binaryTarget(
            name: "MapLibre",
            path: "Artifacts/MapLibre.xcframework"
        ),
    ]
)
