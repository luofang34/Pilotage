// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "PilotageMapLibreTerrain",
    platforms: [
        .iOS(.v26),
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
