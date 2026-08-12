// swift-tools-version: 6.2

import Foundation
import PackageDescription

let terrainRendererEnabled = ProcessInfo.processInfo.environment[
    "PILOTAGE_MAPLIBRE_TERRAIN"
] == "1"

let mapLibreDependency: Package.Dependency = if terrainRendererEnabled {
    .package(path: "../PilotageMapLibreTerrain")
} else {
    .package(
        url: "https://github.com/maplibre/maplibre-gl-native-distribution",
        exact: "6.28.0"
    )
}

let mapLibrePackage = terrainRendererEnabled
    ? "PilotageMapLibreTerrain"
    : "maplibre-gl-native-distribution"

let package = Package(
    name: "PilotageMapLibreBinding",
    platforms: [
        .iOS(.v26),
    ],
    products: [
        .library(name: "PilotageMapLibreBinding", targets: ["PilotageMapLibreBinding"]),
    ],
    dependencies: [
        mapLibreDependency,
        .package(path: "../PilotageGeoJSONEdge"),
        .package(path: "../PilotageSituationCore"),
    ],
    targets: [
        .target(
            name: "PilotageMapLibreBinding",
            dependencies: [
                .product(name: "PilotageGeoJSONEdge", package: "PilotageGeoJSONEdge"),
                .product(name: "PilotageSituationCore", package: "PilotageSituationCore"),
                .product(name: "MapLibre", package: mapLibrePackage),
            ],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
