#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-control-feel-boundary.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/adapters/aviate/src/uplink"

printf '%s\n' 'const MAX_DT_S: f32 = 0.1;' > "$fixture/adapters/aviate/src/uplink.rs"
printf '%s\n' 'pub fn shape(value: f32) -> f32 { value }' \
    > "$fixture/adapters/aviate/src/uplink/feel.rs"
bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" >/dev/null

printf '%s\n' 'const MAX_TAKEOFF_THRUST: f32 = 0.75;' \
    >> "$fixture/adapters/aviate/src/uplink/feel.rs"
output="$fixture/failure.txt"
if bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" \
    >"$output" 2>&1; then
    echo "the Aviate control-feel guard accepted a response constant" >&2
    exit 1
fi
if ! grep -Fq 'MAX_TAKEOFF_THRUST' "$output"; then
    echo "the Aviate control-feel guard did not identify the response constant" >&2
    exit 1
fi

echo "Aviate control-feel boundary self-test: OK"
