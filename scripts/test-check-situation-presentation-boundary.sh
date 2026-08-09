#!/usr/bin/env bash
# Test each failure path in the situation presentation boundary check.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-presentation-boundary.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

rust_root="$fixture/crates/pilotage-presentation/src"
swift_root="$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/Binding"
mkdir -p "$rust_root" "$swift_root"
printf '%s\n' 'pub struct DisplayValue;' > "$rust_root/lib.rs"
printf '%s\n' 'struct DisplayValue {}' > "$swift_root/DisplayValue.swift"

bash "$repo_root/scripts/check-situation-presentation-boundary.sh" "$fixture" >/dev/null

printf '%s\n' 'pub const ENGINE: &str = "MapLibre";' > "$rust_root/backend.rs"
if bash "$repo_root/scripts/check-situation-presentation-boundary.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: a display implementation entered the Rust adapter" >&2
    exit 1
fi
rm "$rust_root/backend.rs"

printf '%s\n' 'struct TrackSnapshot {}' > "$swift_root/Domain.swift"
if bash "$repo_root/scripts/check-situation-presentation-boundary.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: a domain snapshot type entered the Swift binding" >&2
    exit 1
fi

echo "situation presentation boundary self-test: OK"
