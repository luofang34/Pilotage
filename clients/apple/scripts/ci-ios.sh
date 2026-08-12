#!/bin/sh
# Verify the Rust facade and the generated Swift package.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
manifest="$client_root/rust/pilotage-situation-ffi/Cargo.toml"
package="$client_root/Packages/PilotageCore"
map_package="$client_root/Packages/PilotageMapLibreBinding"
geojson_package="$client_root/Packages/PilotageGeoJSONEdge"
radio_package="$client_root/Packages/PilotageRadioSource"
cache_root="$client_root/.build/cache"
mkdir -p "$cache_root/clang" "$cache_root/swiftpm"
export CLANG_MODULE_CACHE_PATH="$cache_root/clang"
export SWIFTPM_MODULECACHE_OVERRIDE="$cache_root/clang"
export XDG_CACHE_HOME="$cache_root/swiftpm"

case "${PILOTAGE_MAPLIBRE_TERRAIN:-0}" in
    0) ;;
    1) sh "$client_root/scripts/build-maplibre-terrain.sh" ;;
    *)
        echo "PILOTAGE_MAPLIBRE_TERRAIN must be 0 or 1" >&2
        exit 2
        ;;
esac

sh "$client_root/scripts/prepare-radio-sources.sh"
cargo fmt --manifest-path "$manifest" --check
if grep -RInE 'anyhow(::Error)?' "$client_root/rust/pilotage-situation-ffi/src"; then
    echo "the hand-written FFI facade must use typed errors" >&2
    exit 1
fi
cargo clippy --manifest-path "$manifest" --locked --all-targets -- -D warnings
cargo test --manifest-path "$manifest" --locked
sh "$client_root/scripts/build-xcframework.sh"
swift build --disable-sandbox --package-path "$package"
swift test --disable-sandbox --package-path "$package"
swift test --disable-sandbox --package-path "$geojson_package"
swift test --disable-sandbox --package-path "$radio_package"
simulator_sdk=$(xcrun --sdk iphonesimulator --show-sdk-path)
swift build \
    --disable-sandbox \
    --package-path "$map_package" \
    --scratch-path "$client_root/.build/maplibre-package" \
    --triple arm64-apple-ios18.0-simulator \
    --sdk "$simulator_sdk" \
    --target PilotageMapLibreBinding

if [ -n "${AERO_LINK_SOURCE:-}" ]; then
    sh "$client_root/scripts/generate-project.sh"
    sh "$client_root/scripts/check-driver-coexistence.sh"
    xcodebuild \
        -project "$client_root/Pilotage.xcodeproj" \
        -scheme Pilotage \
        -destination 'generic/platform=iOS Simulator' \
        -derivedDataPath "$client_root/.build/app-derived-data" \
        ARCHS=arm64 \
        CODE_SIGNING_ALLOWED=NO \
        build
fi
