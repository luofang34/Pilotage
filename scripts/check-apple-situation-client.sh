#!/usr/bin/env bash
# Verify the fixed dependencies and the private driver identity boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
client="$root/clients/apple-situation"
status=0

require_pattern() {
    local pattern=$1
    local file=$2
    local message=$3
    if ! grep -Eq "$pattern" "$file"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

revision=$(tr -d '[:space:]' < "$client/AERO_LINK_REVISION")
if [[ ! "$revision" =~ ^[0-9a-f]{40}$ ]]; then
    echo "FORBIDDEN: AERO_LINK_REVISION must contain one full commit identity" >&2
    status=1
fi

require_pattern 'exact: "6\.28\.0"' \
    "$client/Packages/PilotageMapLibreBinding/Package.swift" \
    "the MapLibre Native package must use the reviewed exact version"
require_pattern 'maplibre/maplibre-gl-native-distribution' \
    "$client/Packages/PilotageMapLibreBinding/Package.swift" \
    "the binding must use the official MapLibre Native distribution"
require_pattern 'AeroLink/AeroLinkAppleClient' "$client/project.yml" \
    "the application must embed its AeroLink client copy"
require_pattern 'AeroLink/AeroLinkDriver' "$client/project.yml" \
    "the application must embed its AeroLink driver copy"
require_pattern 'com\.apple\.developer\.driverkit\.communicates-with-drivers' \
    "$client/App/PilotageSituation.entitlements" \
    "the host must declare its DriverKit communication capability"
require_pattern 'com\.apple\.developer\.system-extension\.install' \
    "$client/App/PilotageSituation.entitlements" \
    "the host must declare its system extension capability"
require_pattern 'AeroLinkDriverBundleIdentifier' "$client/App/Info.plist" \
    "the host must identify its embedded driver"
require_pattern '"[$]AERO_LINK_HOST_BUNDLE_IDENTIFIER"[.][*]' \
    "$client/scripts/generate-project.sh" \
    "the project generator must require a nested driver App ID"
require_pattern 'brew install xcodegen' \
    "$root/.github/workflows/ci.yml" \
    "the Apple situation client job must install XcodeGen"
require_pattern 'ARCHS=arm64' \
    "$client/scripts/ci-ios.sh" \
    "the simulator build must use the available Rust architecture"

if [ "$status" -ne 0 ]; then
    echo "Apple situation client: FAILED" >&2
    exit 1
fi

echo "Apple situation client: OK"
