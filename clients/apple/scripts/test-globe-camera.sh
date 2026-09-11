#!/bin/sh
set -eu
client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
output="$client_root/.build/globe-camera-tests"
mkdir -p "$output/Sources/GlobeCameraUnderTest" "$output/Tests/GlobeCameraTests" "$output/cache"
cat > "$output/Package.swift" <<'SWIFT'
// swift-tools-version: 6.0
import PackageDescription
let package = Package(
    name: "GlobeCameraUnderTest",
    targets: [
        .target(name: "GlobeCameraUnderTest"),
        .testTarget(name: "GlobeCameraTests", dependencies: ["GlobeCameraUnderTest"]),
    ]
)
SWIFT
cp "$client_root/App/GlobeCamera.swift" "$output/Sources/GlobeCameraUnderTest/"
cp "$client_root/Tests/GlobeCameraTests.swift" "$output/Tests/GlobeCameraTests/"
CLANG_MODULE_CACHE_PATH="$output/cache" swift test --disable-sandbox --package-path "$output"
