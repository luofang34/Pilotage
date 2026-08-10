#!/usr/bin/env bash
# Keep the Navdata tile build deterministic, identified, and offline.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
crate="$root/crates/pilotage-navdata-tiles"
manifest="$crate/Cargo.toml"
builder="$crate/src/build.rs"
archive="$crate/src/archive.rs"
reader="$crate/src/reader.rs"
tests="$crate/src/tests.rs"
contract="$root/docs/navdata-baseline-tile-bundle.md"
status=0

require_text() {
    local pattern="$1"
    local file="$2"
    local message="$3"
    if [ ! -f "$file" ] || ! grep -qF "$pattern" "$file"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

if [ -f "$manifest" ] && grep -Eq '^(reqwest|hyper|ureq|wtransport|tokio)[[:space:]]*=' "$manifest"; then
    echo "FORBIDDEN: the Navdata tile crate has a network or async transport dependency" >&2
    status=1
fi

if [ -f "$builder" ] && grep -Eq 'std::fs|fs::|aerocontext_navdata|inspect\(|decode\(' "$builder"; then
    echo "FORBIDDEN: the tile builder reads a source file instead of an identified snapshot" >&2
    status=1
fi

require_text 'snapshot: &IdentifiedNavdataSnapshotV1' "$builder" \
    'the builder must accept one identified Navdata snapshot'
require_text 'pilotage_cycle' "$archive" \
    'the archive must carry the Navdata cycle identity'
require_text 'pilotage_snapshot_id' "$archive" \
    'the archive must carry the snapshot identity'
require_text 'pilotage_snapshot_digest' "$archive" \
    'the archive must carry the snapshot digest'
require_text '("format", "pbf".to_owned())' "$archive" \
    'the archive must identify Mapbox Vector Tile payloads'
require_text 'GzBuilder::new()' "$archive" \
    'the archive must gzip each vector tile deterministically'
require_text 'pub struct OfflineTileReader' "$reader" \
    'the crate must keep a no-network installed-bundle reader'
require_text 'subject_id' "$crate/src/feature.rs" \
    'each drawable feature must carry a stable subject identifier'
require_text 'same_snapshot_produces_identical_archive_bytes' "$tests" \
    'the crate must compare complete archive bytes across two builds'
require_text 'Output SHA-256 from both builds' "$contract" \
    'the contract must record a full-cycle reproducibility measurement'
require_text 'The two output files had identical bytes.' "$contract" \
    'the full-cycle measurement must state the reproducibility result'

if [ "$status" -ne 0 ]; then
    echo "Navdata tile bundle guard: FAILED" >&2
    exit 1
fi

echo "Navdata tile bundle guard: OK"
