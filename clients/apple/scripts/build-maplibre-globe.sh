#!/bin/sh
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fork=${MAPLIBRE_GLOBE_SOURCE:-/private/tmp/maplibre-rs-globe-fork}
revision=$(cat "$client_root/MAPLIBRE_GLOBE_REVISION")
aviation_id=$(shasum -a 256 "$client_root/renderer/aviation/build.json" | cut -c1-12)
source_root=${MAPLIBRE_GLOBE_BUILD_SOURCE:-"$client_root/.build/maplibre-globe/source-$revision-$aviation_id"}
ifr_source=${IFR_MAP_LAB_SOURCE:-"$client_root/../../../ifr-map-lab"}
output="$client_root/.build/maplibre-globe"
resources="$client_root/.build/GlobeResources"
mkdir -p "$output/include" "$resources"

if [ ! -f "$source_root/Cargo.toml" ]; then
    mkdir -p "$source_root"
    git -C "$fork" archive "$revision" > "$output/source.tar"
    tar -xf "$output/source.tar" -C "$source_root"
fi
python3 "$client_root/scripts/prepare-maplibre-aviation.py" \
    --source "$source_root" --ifr-source "$ifr_source"
host="$source_root/apple/visionos"
target_root=${CARGO_TARGET_DIR:-"$output/target"}
if [ ! -f "$host/vendor/wgpu-22.1.0/Cargo.toml" ]; then
    sh "$host/vendor-wgpu.sh"
fi

for target in aarch64-apple-ios aarch64-apple-ios-sim; do
    CARGO_TARGET_DIR="$target_root" IPHONEOS_DEPLOYMENT_TARGET=26.0 \
        cargo +stable build --locked --offline --release \
        --manifest-path "$host/Cargo.toml" \
        --config "$host/.cargo/config.toml" --target "$target"
done

cp "$host/maplibre_visionos.h" "$output/include/"
cp "$host/MapLibreVision/MapLibreVision/style.json" "$resources/GlobeStyle.json"
cp "$source_root/LICENSE-MIT" "$resources/MapLibre-LICENSE-MIT.txt"
cp "$source_root/LICENSE-APACHE" "$resources/MapLibre-LICENSE-APACHE.txt"
cp "$client_root/MAPLIBRE_GLOBE_REVISION" "$resources/GlobeRevision.txt"
python3 - "$resources/GlobeStyle.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
style = json.loads(path.read_text())
if style.get("projection", {}).get("type") != "globe":
    raise SystemExit("The map style must use globe projection.")
PY

artifact="$client_root/.build/MapLibreGlobe.xcframework"
rm -rf "$artifact"
xcodebuild -create-xcframework \
    -library "$target_root/aarch64-apple-ios/release/libmaplibre_visionos.a" \
    -headers "$output/include" \
    -library "$target_root/aarch64-apple-ios-sim/release/libmaplibre_visionos.a" \
    -headers "$output/include" \
    -output "$artifact"
