#!/usr/bin/env bash
# Keep platform and domain names on their assigned side of the display boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
rust_root="$root/crates/pilotage-presentation"
swift_root="$root/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources"
traffic_source="$rust_root/src/traffic.rs"
weather_source="$rust_root/src/weather.rs"
status=0

if grep -RInE 'MapLibre|MLN[A-Z]|GeoJSON|UIKit|SwiftUI' "$rust_root"; then
    echo "FORBIDDEN: the portable presentation adapter names a display implementation" >&2
    status=1
fi

if grep -RInE 'pilotage[_-]terrain[_-]query|rusqlite|png::Decoder' "$rust_root/src"; then
    echo "FORBIDDEN: the portable presentation adapter reads a terrain archive" >&2
    status=1
fi

if [ -f "$traffic_source" ] && grep -nE 'surveillance[_-]core' "$traffic_source"; then
    echo "FORBIDDEN: traffic display policy bypasses the typed feature adapter" >&2
    status=1
fi

if [ -f "$traffic_source" ] && ! grep -q 'surveillance_geojson' "$traffic_source"; then
    echo "FORBIDDEN: traffic display policy does not consume typed feature changes" >&2
    status=1
fi

if [ -f "$weather_source" ] && grep -nE \
    'serde(_json)?|WeatherPayload|media_type|from_(slice|str)|ceiling|visibility' \
    "$weather_source"; then
    echo "FORBIDDEN: weather display policy decodes a payload or derives source data" >&2
    status=1
fi

if [ -f "$weather_source" ] && ! grep -q 'airmass_geojson' "$weather_source"; then
    echo "FORBIDDEN: weather display policy does not consume typed feature changes" >&2
    status=1
fi

if [ -d "$swift_root" ] && grep -RInE \
    'airmass[_-]core|surveillance[_-]core|TrackSnapshot|WeatherSnapshot|WeatherProduct' \
    "$swift_root"; then
    echo "FORBIDDEN: the Swift display binding names a domain snapshot type" >&2
    status=1
fi

if [ "$status" -ne 0 ]; then
    echo "situation presentation boundary: FAILED" >&2
    exit 1
fi

echo "situation presentation boundary: OK"
