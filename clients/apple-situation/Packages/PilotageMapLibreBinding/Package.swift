// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "PilotageMapLibreBinding",
    platforms: [
        .iOS(.v18),
    ],
    products: [
        .library(name: "PilotageMapLibreBinding", targets: ["PilotageMapLibreBinding"]),
    ],
    dependencies: [
        .package(
            url: "https://github.com/maplibre/maplibre-gl-native-distribution",
            exact: "6.28.0"
        ),
        .package(path: "../PilotageGeoJSONEdge"),
        .package(path: "../PilotageSituationCore"),
    ],
    targets: [
        .target(
            name: "PilotageMapLibreBinding",
            dependencies: [
                .product(name: "PilotageGeoJSONEdge", package: "PilotageGeoJSONEdge"),
                .product(name: "PilotageSituationCore", package: "PilotageSituationCore"),
                .product(name: "MapLibre", package: "maplibre-gl-native-distribution"),
            ],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
