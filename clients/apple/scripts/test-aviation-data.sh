#!/bin/sh
set -eu
client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root="$client_root/.build/aviation-data-tests"
mkdir -p "$test_root/Sources/AviationDataRecords" "$test_root/Tests/AviationDataRecordsTests"
cp "$client_root/App/AviationDataRecords.swift" "$test_root/Sources/AviationDataRecords/"
cp "$client_root/App/AviationChartStyle.swift" "$test_root/Sources/AviationDataRecords/"
cp "$client_root/App/AviationMapStyle.swift" "$test_root/Sources/AviationDataRecords/"
cp "$client_root/Tests/AviationDataTests.swift" "$test_root/Tests/AviationDataRecordsTests/"
cp "$client_root/Tests/AviationChartStyleTests.swift" "$test_root/Tests/AviationDataRecordsTests/"
cat > "$test_root/Package.swift" <<'SWIFT'
// swift-tools-version: 6.2
import PackageDescription
let package = Package(
    name: "AviationDataRecords",
    platforms: [.macOS(.v15)],
    targets: [
        .target(name: "AviationDataRecords"),
        .testTarget(name: "AviationDataRecordsTests", dependencies: ["AviationDataRecords"]),
    ]
)
SWIFT
CLANG_MODULE_CACHE_PATH="$test_root/cache" swift test --disable-sandbox --package-path "$test_root"
