// swift-tools-version: 6.1

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
