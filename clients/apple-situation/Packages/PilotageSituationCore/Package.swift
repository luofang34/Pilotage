// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "PilotageSituationCore",
    platforms: [
        .iOS(.v18),
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
            dependencies: ["PilotageSituationFFI"]
        ),
        .testTarget(
            name: "PilotageSituationCoreTests",
            dependencies: ["PilotageSituationCore"]
        ),
    ]
)
