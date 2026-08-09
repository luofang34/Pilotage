// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "PilotageAppleInstrumentConsumer",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(
            name: "PilotageAppleInstrumentConsumer",
            targets: ["PilotageAppleInstrumentConsumer"]
        ),
    ],
    dependencies: [
        .package(
            url: "https://github.com/luofang34/IndicateAppleDisplay.git",
            revision: "74e5845d09a58342fd282ec426759550dafc6887"
        ),
    ],
    targets: [
        .target(
            name: "PilotageAppleInstrumentConsumer",
            dependencies: [
                .product(name: "IndicateAppleDisplay", package: "IndicateAppleDisplay"),
            ]
        ),
        .testTarget(
            name: "PilotageAppleInstrumentConsumerTests",
            dependencies: ["PilotageAppleInstrumentConsumer"]
        ),
    ]
)
