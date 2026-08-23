#!/usr/bin/env bash
# Keep Aviate flight-response constants in the typed control-feel artifact.
set -euo pipefail

root_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
uplink_root="$root_dir/adapters/aviate/src/uplink"
uplink_file="$root_dir/adapters/aviate/src/uplink.rs"

if rg -n --glob '!tests.rs' \
    'const[[:space:]]+[A-Z0-9_]*(HORIZONTAL|VERTICAL|YAW|TILT|THRUST|TAKEOFF|ACCEL|JERK|DEADZONE|EXPO|HOLD)[A-Z0-9_]*[[:space:]]*:[[:space:]]*f(32|64)' \
    "$uplink_file" "$uplink_root"; then
    echo "FORBIDDEN: Aviate response constant bypasses the control-feel artifact" >&2
    exit 1
fi

echo "Aviate control-feel boundary: OK"
