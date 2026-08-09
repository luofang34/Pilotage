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
            revision: "ca5fe14f22798fbee2d184970b928b04736f4083"
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
