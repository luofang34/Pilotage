// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "PilotageRadioSource",
    platforms: [
        .iOS(.v26),
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
