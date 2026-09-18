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
            revision: "77304c9523715b7e4a03a7bba10ae04eb7845b94"
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
