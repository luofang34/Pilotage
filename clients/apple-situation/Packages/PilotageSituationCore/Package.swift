// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "PilotageSituationCore",
    platforms: [
        .iOS(.v26),
        .macOS(.v15),
    ],
    products: [
        .library(name: "PilotageSituationCore", targets: ["PilotageSituationCore"]),
    ],
    targets: [
        .binaryTarget(
            name: "PilotageSituationFFI",
            path: "artifacts/PilotageSituationFFI.xcframework"
        ),
        .target(
            name: "PilotageSituationCore",
            dependencies: ["PilotageSituationFFI"],
            linkerSettings: [
                .linkedLibrary("sqlite3"),
            ]
        ),
        .testTarget(
            name: "PilotageSituationCoreTests",
            dependencies: ["PilotageSituationCore"]
        ),
    ]
)
