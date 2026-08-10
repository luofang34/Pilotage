#!/usr/bin/env bash
# Exercise the failure paths in the Navdata tile bundle guard.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-navdata-tiles.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

mkdir -p "$fixture/crates" "$fixture/docs"
cp -R "$repo_root/crates/pilotage-navdata-tiles" "$fixture/crates/"
cp "$repo_root/docs/navdata-baseline-tile-bundle.md" "$fixture/docs/"

guard="$repo_root/scripts/check-navdata-tile-bundle.sh"
bash "$guard" "$fixture" >/dev/null

printf '\nreqwest = "0.12"\n' >> "$fixture/crates/pilotage-navdata-tiles/Cargo.toml"
if bash "$guard" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a network dependency" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-navdata-tiles/Cargo.toml" \
    "$fixture/crates/pilotage-navdata-tiles/Cargo.toml"

sed -i.bak 's/snapshot: &IdentifiedNavdataSnapshotV1/snapshot: \&NavDataSnapshot/' \
    "$fixture/crates/pilotage-navdata-tiles/src/build.rs"
if bash "$guard" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a builder without snapshot identity" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-navdata-tiles/src/build.rs" \
    "$fixture/crates/pilotage-navdata-tiles/src/build.rs"

sed -i.bak '/pilotage_snapshot_digest/d' \
    "$fixture/crates/pilotage-navdata-tiles/src/archive.rs"
if bash "$guard" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted missing digest metadata" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-navdata-tiles/src/archive.rs" \
    "$fixture/crates/pilotage-navdata-tiles/src/archive.rs"

sed -i.bak 's/subject_id/baseline_id/g' \
    "$fixture/crates/pilotage-navdata-tiles/src/feature.rs"
if bash "$guard" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted features without subject identifiers" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-navdata-tiles/src/feature.rs" \
    "$fixture/crates/pilotage-navdata-tiles/src/feature.rs"

sed -i.bak '/same_snapshot_produces_identical_archive_bytes/d' \
    "$fixture/crates/pilotage-navdata-tiles/src/tests.rs"
if bash "$guard" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a missing byte reproducibility test" >&2
    exit 1
fi

echo "Navdata tile bundle guard self-test: OK"
