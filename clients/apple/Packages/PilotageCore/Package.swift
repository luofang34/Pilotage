// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "PilotageCore",
    platforms: [
        .iOS(.v26),
        .macOS(.v15),
    ],
    products: [
        .library(name: "PilotageCore", targets: ["PilotageCore"]),
    ],
    targets: [
        .binaryTarget(
            name: "PilotageFFI",
            path: "artifacts/PilotageFFI.xcframework"
        ),
        .target(
            name: "PilotageCore",
            dependencies: ["PilotageFFI"],
            linkerSettings: [
                .linkedLibrary("sqlite3"),
            ]
        ),
        .testTarget(
            name: "PilotageCoreTests",
            dependencies: ["PilotageCore"]
        ),
    ]
)
