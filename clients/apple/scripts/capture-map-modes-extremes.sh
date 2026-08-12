#!/usr/bin/env bash
# Photograph the map modes panel at the window shapes that break it.
#
# The panel is drawn two ways and the narrow one has two extremes: tall, where a sheet
# left to itself takes the whole screen, and short, where it asks for more height than
# the window has. Neither is reachable on a full screen tablet, and neither shows up in a
# build. A phone in portrait is the tall one and the same phone turned is the short one,
# so both are one command away instead of a message asking somebody to drag a window.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
device="${MAP_MODES_SIM:-}"
out="${1:-${TMPDIR:-/tmp}/map-modes-extremes}"
bundle="org.luofang.pilotage"

if [ -z "$device" ]; then
    device=$(xcrun simctl list devices available \
        | awk '/iPhone/ { match($0, /\(([0-9A-F-]{36})\)/, m); if (m[1] != "") { print m[1]; exit } }' \
        || true)
fi
if [ -z "$device" ]; then
    device=$(xcrun simctl list devices available | grep -m1 -oE '[0-9A-F]{8}-[0-9A-F-]{27}')
fi
[ -n "$device" ] || { echo "no simulator to photograph on" >&2; exit 1; }

mkdir -p "$out"
xcrun simctl bootstatus "$device" -b >/dev/null 2>&1 || true

app="${MAP_MODES_APP:-}"
if [ -z "$app" ]; then
    app=$(find "$here" -maxdepth 6 -path "*Debug-iphonesimulator/Pilotage.app" \
        -print -quit 2>/dev/null || true)
fi
[ -n "$app" ] || { echo "build for a simulator first, or set MAP_MODES_APP" >&2; exit 1; }

xcrun simctl install "$device" "$app" >/dev/null

shoot() {
    local name=$1 orientation=$2
    xcrun devicectl device orientation set --device "$device" "$orientation" >/dev/null 2>&1 || true
    xcrun simctl terminate "$device" "$bundle" >/dev/null 2>&1 || true
    sleep 1
    # The panel opens itself, because there is no way to reach in and press it.
    xcrun simctl launch "$device" "$bundle" -OpenMapModes >/dev/null
    sleep 8
    xcrun simctl io "$device" screenshot "$out/$name.png" >/dev/null
    echo "  $name -> $out/$name.png"
}

echo "map modes, narrow:"
shoot narrow-tall portrait
shoot narrow-short landscapeLeft
xcrun devicectl device orientation set --device "$device" portrait >/dev/null 2>&1 || true
