// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "PilotageGeoJSONEdge",
    products: [
        .library(name: "PilotageGeoJSONEdge", targets: ["PilotageGeoJSONEdge"]),
    ],
    targets: [
        .target(name: "PilotageGeoJSONEdge"),
        .testTarget(
            name: "PilotageGeoJSONEdgeTests",
            dependencies: ["PilotageGeoJSONEdge"]
        ),
    ]
)
