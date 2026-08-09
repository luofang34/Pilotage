// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "PilotageRadioSource",
    platforms: [
        .iOS(.v18),
        .macOS(.v15),
    ],
    products: [
        .library(name: "PilotageRadioSource", targets: ["PilotageRadioSource"]),
    ],
    targets: [
        .target(name: "PilotageRadioSource"),
        .testTarget(
            name: "PilotageRadioSourceTests",
            dependencies: ["PilotageRadioSource"]
        ),
    ]
)
